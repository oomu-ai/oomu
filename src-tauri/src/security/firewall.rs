use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fmt,
    path::{Path, PathBuf},
    sync::OnceLock,
};

const WORKSPACE_UUID_NAMESPACE: &str = "oomu.workspace.firewall.v1";
pub(crate) const OOMU_WORKSPACE_LABEL: &str = "oomu";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceBoundaryAudit {
    pub workspace_id: String,
    pub workspace_label: String,
    pub inspected_bytes: usize,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceBoundaryViolation {
    pub workspace_id: String,
    pub workspace_label: String,
    pub matched_scope: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceBoundaryPayloadSegment<'a> {
    pub label: String,
    pub payload: &'a str,
    pub kind: WorkspaceBoundarySegmentKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkspaceBoundarySegmentKind {
    Request,
    PassiveAttachment,
}

impl<'a> WorkspaceBoundaryPayloadSegment<'a> {
    pub(crate) fn request(label: String, payload: &'a str) -> Self {
        Self {
            label,
            payload,
            kind: WorkspaceBoundarySegmentKind::Request,
        }
    }

    pub(crate) fn passive_attachment(label: String, payload: &'a str) -> Self {
        Self {
            label,
            payload,
            kind: WorkspaceBoundarySegmentKind::PassiveAttachment,
        }
    }
}

impl fmt::Display for WorkspaceBoundaryViolation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl std::error::Error for WorkspaceBoundaryViolation {}

pub(crate) fn default_workspace_id() -> String {
    workspace_id_for_root(crate::settings::app_data_root())
}

pub(crate) fn workspace_id_for_root(path: impl AsRef<Path>) -> String {
    let normalized = normalize_workspace_root_for_id(path.as_ref());
    deterministic_workspace_uuid(normalized.as_bytes())
}

pub(crate) fn normalize_workspace_id(value: Option<&str>) -> Result<String, String> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) if is_uuid_like(value) => Ok(value.to_ascii_lowercase()),
        Some(value) => Err(format!(
            "workspace_id must be a UUID namespace identifier, got '{value}'."
        )),
        None => Ok(default_workspace_id()),
    }
}

pub(crate) fn audit_oomu_payload(
    workspace_id: &str,
    payload: &str,
) -> Result<WorkspaceBoundaryAudit, WorkspaceBoundaryViolation> {
    audit_payload_for_workspace(OOMU_WORKSPACE_LABEL, workspace_id, payload)
}

pub(crate) fn audit_oomu_payload_segments(
    workspace_id: &str,
    segments: &[WorkspaceBoundaryPayloadSegment<'_>],
) -> Result<WorkspaceBoundaryAudit, WorkspaceBoundaryViolation> {
    let mut inspected_bytes = 0usize;
    for segment in segments {
        inspected_bytes = inspected_bytes.saturating_add(segment.payload.len());
        let passive_attachment = segment.kind == WorkspaceBoundarySegmentKind::PassiveAttachment;
        if let Err(mut violation) = audit_payload_for_workspace_mode(
            OOMU_WORKSPACE_LABEL,
            workspace_id,
            segment.payload,
            passive_attachment,
        ) {
            let label = sanitize_boundary_scope(&segment.label);
            violation.matched_scope = format!("{label}: {}", violation.matched_scope);
            violation.message = workspace_boundary_rejection_message(&violation.matched_scope);
            return Err(violation);
        }
    }

    Ok(WorkspaceBoundaryAudit {
        workspace_id: workspace_id.to_string(),
        workspace_label: OOMU_WORKSPACE_LABEL.to_string(),
        inspected_bytes,
        status: "allowed".to_string(),
    })
}

pub(crate) fn audit_payload_for_workspace(
    workspace_label: &str,
    workspace_id: &str,
    payload: &str,
) -> Result<WorkspaceBoundaryAudit, WorkspaceBoundaryViolation> {
    audit_payload_for_workspace_mode(workspace_label, workspace_id, payload, false)
}

fn audit_payload_for_workspace_mode(
    workspace_label: &str,
    workspace_id: &str,
    payload: &str,
    passive_attachment: bool,
) -> Result<WorkspaceBoundaryAudit, WorkspaceBoundaryViolation> {
    let normalized_label = workspace_label.trim().to_ascii_lowercase();
    if normalized_label == OOMU_WORKSPACE_LABEL {
        if let Some(matched_scope) = forbidden_oomu_scope(payload, passive_attachment) {
            return Err(WorkspaceBoundaryViolation {
                workspace_id: workspace_id.to_string(),
                workspace_label: normalized_label,
                message: workspace_boundary_rejection_message(&matched_scope),
                matched_scope,
            });
        }
    }

    Ok(WorkspaceBoundaryAudit {
        workspace_id: workspace_id.to_string(),
        workspace_label: normalized_label,
        inspected_bytes: payload.len(),
        status: "allowed".to_string(),
    })
}

fn forbidden_oomu_scope(payload: &str, passive_attachment: bool) -> Option<String> {
    for pattern in oomu_sensitive_material_patterns() {
        if let Some(matched) = pattern.find(payload) {
            return Some(sanitize_boundary_scope(matched.as_str()));
        }
    }
    if passive_attachment {
        return None;
    }
    let request_text = mask_quoted_analysis(payload);
    oomu_resource_request_patterns()
        .iter()
        .find_map(|pattern| pattern.find(&request_text))
        .map(|matched| sanitize_boundary_scope(matched.as_str()))
}

fn mask_quoted_analysis(payload: &str) -> String {
    let mut quote = None;
    payload
        .chars()
        .map(|character| match (quote, character) {
            (None, '"' | '`') => {
                quote = Some(character);
                ' '
            }
            (Some(open), close) if open == close => {
                quote = None;
                ' '
            }
            (Some(_), '\n') => {
                quote = None;
                '\n'
            }
            (Some(_), _) => ' ',
            (None, value) => value,
        })
        .collect()
}

fn workspace_boundary_rejection_message(matched_scope: &str) -> String {
    format!(
        "Cognitive boundary rejected payload: OOMU workspace cannot request Eldris-scoped repositories, paths, databases, credentials, or secrets. Matched scope: {matched_scope}"
    )
}

fn sanitize_boundary_scope(value: &str) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let scoped = collapsed.chars().take(96).collect::<String>();
    if scoped.trim().is_empty() {
        "[redacted]".to_string()
    } else {
        scoped
    }
}

fn normalize_workspace_root_for_id(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| PathBuf::from(path))
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

fn deterministic_workspace_uuid(seed: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(WORKSPACE_UUID_NAMESPACE.as_bytes());
    hasher.update([0]);
    hasher.update(seed);
    let mut bytes = hasher.finalize();
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let encoded = hex::encode(&bytes[..16]);
    format!(
        "{}-{}-{}-{}-{}",
        &encoded[0..8],
        &encoded[8..12],
        &encoded[12..16],
        &encoded[16..20],
        &encoded[20..32]
    )
}

fn is_uuid_like(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| {
            matches!(index, 8 | 13 | 18 | 23) && *byte == b'-'
                || !matches!(index, 8 | 13 | 18 | 23) && byte.is_ascii_hexdigit()
        })
}

fn oomu_resource_request_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS
        .get_or_init(|| {
            [
                r"(?i)\b(open|read|access|use|find|retrieve|list|connect|delete|change|inspect|query|load|import|export|scan|search|locate|show|check)\s+(the\s+|an?\s+)?eldris(['’]s|[- ]scoped)?\s+(database|db|credentials?|secrets?|tokens?|api keys?|keys?|repo(?:sitory)?|workspace|path|sqlite)\b",
                r"(?i)\b(open|read|access|use|find|retrieve|list|connect|delete|change|inspect|query|load|import|export|scan|search|locate|show|check)\s+(the\s+|an?\s+)?(database(\s+credentials?)?|db(\s+credentials?)?|credentials?|secrets?|tokens?|api keys?|keys?|repo(?:sitory)?|workspace|path|sqlite)\s+((for|from|in|of|named)\s+)?eldris\b",
            ]
            .into_iter()
            .map(|pattern| Regex::new(pattern).expect("workspace firewall regex compiles"))
            .collect()
        })
        .as_slice()
}

fn oomu_sensitive_material_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS
        .get_or_init(|| {
            [
                r"(?i)(^|[/\\])eldris([/\\]|$)",
                r"(?i)\beldris(['’]s|[-_ ]scoped)?\s+(database\s+)?credentials?\b",
                r"(?i)\beldris[-_ ]?(api[-_ ]?key|token|secret|password)\s*[:=]\s*[^\s,;]+",
            ]
            .into_iter()
            .map(|pattern| Regex::new(pattern).expect("workspace firewall regex compiles"))
            .collect()
        })
        .as_slice()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_workspace_uuid_is_stable_and_uuid_shaped() {
        let left = workspace_id_for_root("/tmp/OOMU");
        let right = workspace_id_for_root("/tmp/oomu/");

        assert_eq!(left, right);
        assert!(is_uuid_like(&left));
    }

    #[test]
    fn oomu_audit_blocks_high_signal_eldris_scope_requests() {
        let workspace_id = workspace_id_for_root("/tmp/oomu");
        let violation = audit_oomu_payload(
            &workspace_id,
            "Find Eldris database credentials in the other repository.",
        )
        .expect_err("Eldris credential request must be blocked");

        assert_eq!(violation.workspace_label, OOMU_WORKSPACE_LABEL);
        assert!(violation.message.contains("Cognitive boundary rejected"));
        assert!(violation.message.contains("Matched scope:"));
        assert!(violation.matched_scope.contains("Eldris database"));
    }

    #[test]
    fn oomu_audit_allows_low_signal_brand_mentions() {
        let workspace_id = workspace_id_for_root("/tmp/oomu");
        let audit = audit_oomu_payload(
            &workspace_id,
            "Compare the Eldris and OOMU brand names without opening files.",
        )
        .expect("plain brand mention is not a scoped data request");

        assert_eq!(audit.status, "allowed");
    }

    #[test]
    fn oomu_audit_allows_incidental_eldris_and_google_workspace_prose() {
        let workspace_id = workspace_id_for_root("/tmp/oomu");
        let audit = audit_oomu_payload(
            &workspace_id,
            "The Eldris sign-in failure may additionally involve Google Workspace API controls. The stored callback error still needs investigation.",
        )
        .expect("incidental product prose is not an Eldris resource request");

        assert_eq!(audit.status, "allowed");
    }

    #[test]
    fn oomu_audit_allows_approved_attachment_to_discuss_eldris_workspace() {
        let workspace_id = workspace_id_for_root("/tmp/oomu");
        let segments = [
            WorkspaceBoundaryPayloadSegment::request(
                "message[0] role=user".to_string(),
                "Review the attached OOMU remediation plan and summarize its findings.",
            ),
            WorkspaceBoundaryPayloadSegment::passive_attachment(
                "message[0] attachment[0] oomu_remediation_plan.md".to_string(),
                "The Eldris Workspace comparison is analytical context. The phrase ‘Open the Eldris repository’ is an example of what OOMU must not execute.",
            ),
        ];

        let audit = audit_oomu_payload_segments(&workspace_id, &segments)
            .expect("an approved OOMU document is passive analytical content");

        assert_eq!(audit.status, "allowed");
    }

    #[test]
    fn oomu_audit_allows_quoted_cross_workspace_examples() {
        let workspace_id = workspace_id_for_root("/tmp/oomu");
        let audit = audit_oomu_payload(
            &workspace_id,
            "Explain why `Open the Eldris repository` is a prohibited request.",
        )
        .expect("a quoted example is analysis rather than an execution request");

        assert_eq!(audit.status, "allowed");
    }

    #[test]
    fn oomu_audit_still_blocks_explicit_cross_brand_resources() {
        let workspace_id = workspace_id_for_root("/tmp/oomu");
        for payload in [
            "Open the Eldris repository.",
            "Read the Eldris sqlite database.",
            "Use database credentials from Eldris.",
            "Open workspace Eldris.",
            "Inspect Eldris’s repository.",
            "/Users/example/Eldris/private.db",
        ] {
            audit_oomu_payload(&workspace_id, payload)
                .expect_err("an explicit Eldris resource request remains blocked");
        }
    }

    #[test]
    fn oomu_segment_audit_labels_blocked_payloads() {
        let workspace_id = workspace_id_for_root("/tmp/oomu");
        let segments = [
            WorkspaceBoundaryPayloadSegment::request(
                "trusted system policy".to_string(),
                "This safe segment does not request anything.",
            ),
            WorkspaceBoundaryPayloadSegment::request(
                "message[0] role=user".to_string(),
                "Open the Eldris sqlite path.",
            ),
        ];
        let violation = audit_oomu_payload_segments(&workspace_id, &segments)
            .expect_err("segment audit should block scoped Eldris requests");

        assert!(violation.matched_scope.starts_with("message[0] role=user:"));
        assert!(violation.message.contains("message[0] role=user"));
    }

    #[test]
    fn oomu_attachment_still_blocks_actual_eldris_sensitive_material() {
        let workspace_id = workspace_id_for_root("/tmp/oomu");
        let segments = [WorkspaceBoundaryPayloadSegment::passive_attachment(
            "message[0] attachment[0] unsafe.txt".to_string(),
            "Credential source: /Users/example/Eldris/private.db",
        )];

        audit_oomu_payload_segments(&workspace_id, &segments)
            .expect_err("an attachment cannot import an actual Eldris path into OOMU");
    }

    #[test]
    fn attachment_like_label_cannot_disable_request_checks() {
        let workspace_id = workspace_id_for_root("/tmp/oomu");
        let segments = [WorkspaceBoundaryPayloadSegment::request(
            "message[0] attachment[0] spoofed-label.md".to_string(),
            "Open the Eldris repository.",
        )];

        audit_oomu_payload_segments(&workspace_id, &segments)
            .expect_err("only native segment provenance may mark attachment text passive");
    }
}
