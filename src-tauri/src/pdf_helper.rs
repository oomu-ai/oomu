use crate::pdf_protocol::{
    known_failure_code, PdfExtractionFailure, PdfExtractionLimits, PdfExtractionMetrics,
    PdfExtractionRequestHeader, PdfExtractionResponse, PdfExtractionResult,
    MAX_REQUEST_HEADER_BYTES, MAX_RESPONSE_BYTES, PDF_HELPER_PROTOCOL_VERSION, PDF_PARSER_NAME,
    PDF_PARSER_VERSION,
};
use lopdf::{Dictionary, Document, Object};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

struct HelperFailure {
    code: &'static str,
    limit: Option<&'static str>,
}

impl HelperFailure {
    const fn new(code: &'static str) -> Self {
        Self { code, limit: None }
    }

    const fn limit(code: &'static str, limit: &'static str) -> Self {
        Self {
            code,
            limit: Some(limit),
        }
    }
}

#[derive(Default)]
struct Inspection {
    object_count: usize,
    maximum_nesting_depth: usize,
    decompressed_bytes: u64,
    image_count: usize,
}

pub fn run() -> i32 {
    let started = Instant::now();
    let request_and_input = read_request(std::io::stdin().lock());
    let (request, input) = match request_and_input {
        Ok(value) => value,
        Err(failure) => {
            write_response(failure_response(
                String::new(),
                PdfExtractionMetrics::default(),
                failure,
            ));
            return 0;
        }
    };
    let watchdog_done = Arc::new(AtomicBool::new(false));
    start_parent_watchdog(request.parent_pid, &watchdog_done);

    let mut metrics = PdfExtractionMetrics {
        input_bytes: input.len() as u64,
        ..PdfExtractionMetrics::default()
    };
    if let Err(failure) = apply_platform_sandbox() {
        metrics.wall_time_ms = elapsed_ms(started);
        write_response(failure_response(request.request_id, metrics, failure));
        watchdog_done.store(true, Ordering::Release);
        return 0;
    }

    let result = std::panic::catch_unwind(|| extract_document(input.as_slice(), request.limits));
    metrics.wall_time_ms = elapsed_ms(started);
    populate_process_metrics(&mut metrics);
    let response = match result {
        Ok(Ok((extraction, inspection))) => {
            metrics.object_count = inspection.object_count;
            metrics.page_count = extraction.page_count;
            metrics.maximum_nesting_depth = inspection.maximum_nesting_depth;
            metrics.decompressed_bytes = inspection.decompressed_bytes;
            metrics.image_count = inspection.image_count;
            PdfExtractionResponse {
                protocol_version: PDF_HELPER_PROTOCOL_VERSION,
                request_id: request.request_id,
                parser_name: PDF_PARSER_NAME.to_string(),
                parser_version: PDF_PARSER_VERSION.to_string(),
                result: Some(extraction),
                error: None,
                metrics,
            }
        }
        Ok(Err((failure, inspection))) => {
            metrics.object_count = inspection.object_count;
            metrics.maximum_nesting_depth = inspection.maximum_nesting_depth;
            metrics.decompressed_bytes = inspection.decompressed_bytes;
            metrics.image_count = inspection.image_count;
            failure_response(request.request_id, metrics, failure)
        }
        Err(_) => failure_response(
            request.request_id,
            metrics,
            HelperFailure::new("internal_failure"),
        ),
    };
    write_response(response);
    watchdog_done.store(true, Ordering::Release);
    0
}

fn read_request(
    mut input: impl Read,
) -> Result<(PdfExtractionRequestHeader, Zeroizing<Vec<u8>>), HelperFailure> {
    let mut header_length_bytes = [0_u8; 4];
    input
        .read_exact(&mut header_length_bytes)
        .map_err(|_| HelperFailure::new("invalid_request"))?;
    let header_length = u32::from_be_bytes(header_length_bytes) as usize;
    if header_length == 0 || header_length > MAX_REQUEST_HEADER_BYTES {
        return Err(HelperFailure::new("invalid_request"));
    }
    let mut header = vec![0_u8; header_length];
    input
        .read_exact(&mut header)
        .map_err(|_| HelperFailure::new("invalid_request"))?;
    let request: PdfExtractionRequestHeader =
        serde_json::from_slice(&header).map_err(|_| HelperFailure::new("invalid_request"))?;
    if request.protocol_version != PDF_HELPER_PROTOCOL_VERSION
        || request.operation != "extract_text"
        || request.request_id.len() != 64
        || !request
            .request_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || request.input_sha256.len() != 64
        || !request
            .input_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || request.input_byte_count == 0
        || !request.limits.is_exact_secure_profile()
    {
        return Err(HelperFailure::new("invalid_request"));
    }
    if request.input_byte_count > request.limits.input_bytes {
        return Err(HelperFailure::limit("input_limit_exceeded", "input_bytes"));
    }
    let input_length = usize::try_from(request.input_byte_count)
        .map_err(|_| HelperFailure::new("invalid_request"))?;
    let mut document = Zeroizing::new(vec![0_u8; input_length]);
    input
        .read_exact(document.as_mut_slice())
        .map_err(|_| HelperFailure::new("invalid_request"))?;
    let mut trailing = [0_u8; 1];
    if input.read(&mut trailing).unwrap_or(1) != 0 {
        return Err(HelperFailure::new("invalid_request"));
    }
    if hex::encode(Sha256::digest(document.as_slice())) != request.input_sha256 {
        return Err(HelperFailure::new("input_integrity_failed"));
    }
    Ok((request, document))
}

fn extract_document(
    input: &[u8],
    limits: PdfExtractionLimits,
) -> Result<(PdfExtractionResult, Inspection), (HelperFailure, Inspection)> {
    let mut inspection = Inspection::default();
    let document = Document::load_mem(input).map_err(|_| {
        (
            HelperFailure::new("malformed_document"),
            Inspection::default(),
        )
    })?;
    if document.is_encrypted() {
        return Err((
            HelperFailure::new("encrypted_document_unsupported"),
            inspection,
        ));
    }
    if let Err(failure) = inspect_document(&document, limits, &mut inspection) {
        return Err((failure, inspection));
    }

    let pages = document.get_pages();
    if pages.len() > limits.page_count {
        return Err((
            HelperFailure::limit("page_limit_exceeded", "page_count"),
            inspection,
        ));
    }
    let mut text = String::new();
    let mut truncated = false;
    for page_number in pages.keys() {
        let page_text = match document.extract_text(&[*page_number]) {
            Ok(text) => text,
            Err(_) => return Err((HelperFailure::new("text_extraction_failed"), inspection)),
        };
        if !text.is_empty() && !page_text.is_empty() {
            append_bounded(&mut text, "\n", limits.output_text_bytes, &mut truncated);
        }
        append_bounded(
            &mut text,
            &page_text,
            limits.output_text_bytes,
            &mut truncated,
        );
        if truncated {
            break;
        }
    }
    Ok((
        PdfExtractionResult {
            page_count: pages.len(),
            text,
            truncated,
        },
        inspection,
    ))
}

fn inspect_document(
    document: &Document,
    limits: PdfExtractionLimits,
    inspection: &mut Inspection,
) -> Result<(), HelperFailure> {
    if document.objects.len() > limits.object_count {
        return Err(HelperFailure::limit(
            "object_limit_exceeded",
            "object_count",
        ));
    }
    let mut stack = document
        .objects
        .values()
        .map(|object| (object, 1_usize))
        .collect::<Vec<_>>();
    while let Some((object, depth)) = stack.pop() {
        inspection.object_count = inspection.object_count.saturating_add(1);
        inspection.maximum_nesting_depth = inspection.maximum_nesting_depth.max(depth);
        if inspection.object_count > limits.object_count {
            return Err(HelperFailure::limit(
                "object_limit_exceeded",
                "object_count",
            ));
        }
        if depth > limits.nesting_depth {
            return Err(HelperFailure::limit(
                "nesting_limit_exceeded",
                "nesting_depth",
            ));
        }
        match object {
            Object::Array(values) => {
                stack.extend(values.iter().map(|value| (value, depth.saturating_add(1))));
            }
            Object::Dictionary(dictionary) => {
                stack.extend(
                    dictionary
                        .iter()
                        .map(|(_, value)| (value, depth.saturating_add(1))),
                );
            }
            Object::Stream(stream) => {
                let decoded_image_bytes =
                    inspect_image(document, &stream.dict, limits, inspection)?;
                stack.extend(
                    stream
                        .dict
                        .iter()
                        .map(|(_, value)| (value, depth.saturating_add(1))),
                );
                let expanded_bytes = if let Some(decoded_image_bytes) = decoded_image_bytes {
                    // lopdf text extraction never decodes image payloads. Account
                    // conservatively for a four-byte decoded pixel without
                    // invoking DCT/JPX/JBIG2 parsers in this helper.
                    decoded_image_bytes
                } else {
                    stream
                        .decompressed_content()
                        .map_err(|_| HelperFailure::new("malformed_document"))?
                        .len() as u64
                };
                inspection.decompressed_bytes =
                    inspection.decompressed_bytes.saturating_add(expanded_bytes);
                if inspection.decompressed_bytes > limits.decompressed_bytes {
                    return Err(HelperFailure::limit(
                        "decompression_limit_exceeded",
                        "decompressed_bytes",
                    ));
                }
            }
            Object::Null
            | Object::Boolean(_)
            | Object::Integer(_)
            | Object::Real(_)
            | Object::Name(_)
            | Object::String(_, _)
            | Object::Reference(_) => {}
        }
    }
    Ok(())
}

fn inspect_image(
    document: &Document,
    dictionary: &Dictionary,
    limits: PdfExtractionLimits,
    inspection: &mut Inspection,
) -> Result<Option<u64>, HelperFailure> {
    let is_image = dictionary
        .get(b"Subtype")
        .ok()
        .and_then(|value| resolve_object(document, value))
        .and_then(|value| value.as_name().ok())
        .is_some_and(|name| name == b"Image");
    if !is_image {
        return Ok(None);
    }
    inspection.image_count = inspection.image_count.saturating_add(1);
    let width = dictionary_integer(document, dictionary, b"Width").unwrap_or_default();
    let height = dictionary_integer(document, dictionary, b"Height").unwrap_or_default();
    if width <= 0 || height <= 0 {
        return Err(HelperFailure::new("malformed_document"));
    }
    let width = u64::try_from(width).unwrap_or(u64::MAX);
    let height = u64::try_from(height).unwrap_or(u64::MAX);
    if width > limits.image_dimension as u64
        || height > limits.image_dimension as u64
        || width.saturating_mul(height) > limits.image_pixels
    {
        return Err(HelperFailure::limit(
            "image_limit_exceeded",
            "image_dimensions",
        ));
    }
    Ok(Some(width.saturating_mul(height).saturating_mul(4)))
}

fn dictionary_integer(document: &Document, dictionary: &Dictionary, key: &[u8]) -> Option<i64> {
    dictionary
        .get(key)
        .ok()
        .and_then(|value| resolve_object(document, value))
        .and_then(|value| value.as_i64().ok())
}

fn resolve_object<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Object> {
    match object {
        Object::Reference(id) => document.get_object(*id).ok(),
        _ => Some(object),
    }
}

fn append_bounded(output: &mut String, value: &str, limit: usize, truncated: &mut bool) {
    if output.len() >= limit {
        *truncated |= !value.is_empty();
        return;
    }
    let remaining = limit - output.len();
    if value.len() <= remaining {
        output.push_str(value);
        return;
    }
    let mut boundary = remaining;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    output.push_str(&value[..boundary]);
    *truncated = true;
}

fn failure_response(
    request_id: String,
    mut metrics: PdfExtractionMetrics,
    failure: HelperFailure,
) -> PdfExtractionResponse {
    let failure_code = known_failure_code(failure.code).unwrap_or("internal_failure");
    metrics.limit_triggered = failure.limit.map(ToString::to_string);
    PdfExtractionResponse {
        protocol_version: PDF_HELPER_PROTOCOL_VERSION,
        request_id,
        parser_name: PDF_PARSER_NAME.to_string(),
        parser_version: PDF_PARSER_VERSION.to_string(),
        result: None,
        error: Some(PdfExtractionFailure {
            code: failure_code.to_string(),
            message: "The document could not be safely processed.".to_string(),
        }),
        metrics,
    }
}

fn write_response(response: PdfExtractionResponse) {
    if let Ok(bytes) = serde_json::to_vec(&response) {
        if bytes.len() <= MAX_RESPONSE_BYTES {
            let mut output = std::io::stdout().lock();
            let _ = output.write_all(&bytes);
            let _ = output.flush();
        }
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(unix)]
fn start_parent_watchdog(expected_parent: u32, done: &Arc<AtomicBool>) {
    let done = Arc::clone(done);
    thread::spawn(move || {
        while !done.load(Ordering::Acquire) {
            let current_parent = unsafe { libc::getppid() } as u32;
            if current_parent != expected_parent || current_parent <= 1 {
                unsafe { libc::_exit(70) };
            }
            thread::sleep(Duration::from_millis(25));
        }
    });
}

#[cfg(not(unix))]
fn start_parent_watchdog(_expected_parent: u32, _done: &Arc<AtomicBool>) {}

#[cfg(target_os = "macos")]
fn apply_platform_sandbox() -> Result<(), HelperFailure> {
    use std::{ffi::CString, os::raw::c_char, ptr};

    #[link(name = "sandbox")]
    unsafe extern "C" {
        fn sandbox_init(profile: *const c_char, flags: u64, error_buffer: *mut *mut c_char) -> i32;
        fn sandbox_free_error(error_buffer: *mut c_char);
    }

    let profile = CString::new(
        "(version 1)\n\
         (allow default)\n\
         (deny file-read*)\n\
         (deny file-write*)\n\
         (deny network*)\n\
         (deny process-exec)\n\
         (deny process-fork)",
    )
    .map_err(|_| HelperFailure::new("sandbox_unavailable"))?;
    let mut error_buffer = ptr::null_mut();
    let status = unsafe { sandbox_init(profile.as_ptr(), 0, &mut error_buffer) };
    if !error_buffer.is_null() {
        unsafe { sandbox_free_error(error_buffer) };
    }
    if status != 0 {
        return Err(HelperFailure::new("sandbox_unavailable"));
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn apply_platform_sandbox() -> Result<(), HelperFailure> {
    Ok(())
}

#[cfg(unix)]
fn populate_process_metrics(metrics: &mut PdfExtractionMetrics) {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return;
    }
    let usage = unsafe { usage.assume_init() };
    let user_us = (usage.ru_utime.tv_sec as u64)
        .saturating_mul(1_000_000)
        .saturating_add(usage.ru_utime.tv_usec as u64);
    let system_us = (usage.ru_stime.tv_sec as u64)
        .saturating_mul(1_000_000)
        .saturating_add(usage.ru_stime.tv_usec as u64);
    metrics.cpu_time_ms = user_us.saturating_add(system_us) / 1_000;
    #[cfg(target_os = "macos")]
    {
        metrics.peak_memory_bytes = usage.ru_maxrss.max(0) as u64;
    }
    #[cfg(not(target_os = "macos"))]
    {
        metrics.peak_memory_bytes = (usage.ru_maxrss.max(0) as u64).saturating_mul(1_024);
    }
}

#[cfg(not(unix))]
fn populate_process_metrics(_metrics: &mut PdfExtractionMetrics) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_truncation_keeps_utf8_boundary() {
        let mut output = String::new();
        let mut truncated = false;
        append_bounded(&mut output, "abc💚def", 6, &mut truncated);
        assert_eq!(output, "abc");
        assert!(truncated);
    }

    #[test]
    fn only_the_exact_reviewed_limit_profile_is_accepted() {
        let mut limits = PdfExtractionLimits::secure_default();
        assert!(limits.is_exact_secure_profile());
        limits.page_count += 1;
        assert!(!limits.is_exact_secure_profile());
    }
}
