use serde::{Deserialize, Serialize};

pub const PDF_HELPER_PROTOCOL_VERSION: u16 = 1;
pub const PDF_PARSER_NAME: &str = "lopdf";
pub const PDF_PARSER_VERSION: &str = "0.42.0";
pub const MAX_REQUEST_HEADER_BYTES: usize = 4 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PdfExtractionLimits {
    pub input_bytes: u64,
    pub wall_time_ms: u64,
    pub cpu_time_seconds: u64,
    pub address_space_bytes: u64,
    pub output_text_bytes: usize,
    pub page_count: usize,
    pub object_count: usize,
    pub nesting_depth: usize,
    pub decompressed_bytes: u64,
    pub image_dimension: u32,
    pub image_pixels: u64,
}

impl PdfExtractionLimits {
    pub const fn secure_default() -> Self {
        Self {
            input_bytes: 8 * 1024 * 1024,
            wall_time_ms: 5_000,
            cpu_time_seconds: 3,
            address_space_bytes: 256 * 1024 * 1024,
            output_text_bytes: 128 * 1024,
            page_count: 128,
            object_count: 50_000,
            nesting_depth: 32,
            decompressed_bytes: 64 * 1024 * 1024,
            image_dimension: 8_192,
            image_pixels: 40_000_000,
        }
    }

    pub fn is_exact_secure_profile(self) -> bool {
        self == Self::secure_default()
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfExtractionRequestHeader {
    pub protocol_version: u16,
    pub request_id: String,
    pub operation: String,
    pub input_byte_count: u64,
    pub input_sha256: String,
    pub parent_pid: u32,
    pub limits: PdfExtractionLimits,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PdfExtractionResult {
    pub page_count: usize,
    pub text: String,
    pub truncated: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PdfExtractionMetrics {
    pub wall_time_ms: u64,
    pub cpu_time_ms: u64,
    pub peak_memory_bytes: u64,
    pub input_bytes: u64,
    pub object_count: usize,
    pub page_count: usize,
    pub maximum_nesting_depth: usize,
    pub decompressed_bytes: u64,
    pub image_count: usize,
    pub limit_triggered: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PdfExtractionFailure {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PdfExtractionResponse {
    pub protocol_version: u16,
    pub request_id: String,
    pub parser_name: String,
    pub parser_version: String,
    pub result: Option<PdfExtractionResult>,
    pub error: Option<PdfExtractionFailure>,
    pub metrics: PdfExtractionMetrics,
}

pub fn known_failure_code(code: &str) -> Option<&'static str> {
    Some(match code {
        "input_limit_exceeded" => "input_limit_exceeded",
        "invalid_request" => "invalid_request",
        "input_integrity_failed" => "input_integrity_failed",
        "sandbox_unavailable" => "sandbox_unavailable",
        "malformed_document" => "malformed_document",
        "encrypted_document_unsupported" => "encrypted_document_unsupported",
        "page_limit_exceeded" => "page_limit_exceeded",
        "object_limit_exceeded" => "object_limit_exceeded",
        "nesting_limit_exceeded" => "nesting_limit_exceeded",
        "decompression_limit_exceeded" => "decompression_limit_exceeded",
        "image_limit_exceeded" => "image_limit_exceeded",
        "text_extraction_failed" => "text_extraction_failed",
        "internal_failure" => "internal_failure",
        _ => return None,
    })
}
