use super::PresentationIr;
use crate::p0_contracts::EvidenceClass;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreatePresentationRequest {
    pub project_id: String,
    pub task_id: String,
    pub task_run_id: String,
    pub title: String,
    pub presentation: PresentationIr,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PresentationListRequest {
    #[serde(default)]
    pub project_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetPresentationReviewRequest {
    pub presentation_id: String,
    #[serde(default)]
    pub revision: Option<u32>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetPresentationPreviewRequest {
    pub presentation_id: String,
    pub revision: u32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecheckPresentationRequest {
    pub presentation_id: String,
    pub expected_revision: u32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RevisePresentationScopeRequest {
    pub presentation_id: String,
    pub expected_revision: u32,
    pub scope: PresentationRevisionScope,
    #[serde(default)]
    pub target_slide_ids: Vec<String>,
    #[serde(default)]
    pub target_object_ids: Vec<String>,
    pub change_summary: String,
    pub presentation: PresentationIr,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PresentationRevisionScope {
    Slide,
    Element,
    NarrativeSection,
    Notes,
    Citations,
    Theme,
    WholePresentation,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChoosePresentationExportDestinationRequest {
    pub presentation_id: String,
    pub revision: u32,
    pub suggested_name: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportPresentationRevisionRequest {
    pub presentation_id: String,
    pub revision: u32,
    pub grant_token: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InspectPresentationTemplateRequest {
    pub project_id: String,
    pub task_id: String,
    pub task_run_id: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisteredPresentationTemplate {
    pub template_id: String,
    pub name: String,
    pub fingerprint_sha256: String,
    pub master_parts: Vec<String>,
    pub layout_parts: Vec<String>,
    pub slide_count: usize,
    pub exact_part_preservation_supported: bool,
    pub task_summary_compatible: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationExportGrant {
    pub grant_token: String,
    pub display_name: String,
    pub expires_at_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationExportResult {
    pub presentation_id: String,
    pub revision: u32,
    pub display_name: String,
    pub sha256: String,
    pub receipt_id: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PresentationStatus {
    Building,
    CheckRequired,
    Ready,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PresentationCheckerStatus {
    Ready,
    NotInstalled,
    NotQualified,
    AppComponentUnavailable,
    UnsupportedPlatform,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationCheckerReadiness {
    pub status: PresentationCheckerStatus,
    pub required_version: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PresentationIssueSeverity {
    Info,
    Warning,
    Blocker,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PresentationVerificationCheck {
    pub code: String,
    pub passed: bool,
    pub detail: String,
    #[serde(default)]
    pub slide_id: Option<String>,
    #[serde(default)]
    pub object_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PresentationReviewIssue {
    pub issue_id: String,
    pub revision: u32,
    #[serde(default)]
    pub slide_id: Option<String>,
    pub code: String,
    pub severity: PresentationIssueSeverity,
    pub message: String,
    #[serde(default)]
    pub object_id: Option<String>,
    #[serde(default)]
    pub evidence_ref: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PresentationVerificationRecord {
    pub package_sha256: String,
    pub structurally_verified: bool,
    pub visually_verified: bool,
    pub exportable: bool,
    pub checked_at_ms: i64,
    #[serde(default)]
    pub renderer: Option<String>,
    pub checks: Vec<PresentationVerificationCheck>,
    pub issues: Vec<PresentationReviewIssue>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationReviewSummary {
    pub presentation_id: String,
    pub project_id: String,
    pub task_id: String,
    pub task_run_id: String,
    pub artifact_id: String,
    pub title: String,
    pub current_revision: u32,
    pub status: PresentationStatus,
    pub slide_count: usize,
    pub issue_count: usize,
    pub blocker_count: usize,
    pub structurally_verified: bool,
    pub visually_verified: bool,
    pub exportable: bool,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationReviewDetail {
    pub summary: PresentationReviewSummary,
    pub selected_revision: u32,
    pub presentation: PresentationIr,
    pub revision_history: Vec<PresentationRevisionSummary>,
    pub filmstrip: Vec<PresentationFilmstripItem>,
    pub issues: Vec<PresentationReviewIssue>,
    pub notes: Vec<PresentationNotesItem>,
    pub citations: Vec<PresentationCitationItem>,
    pub provenance: Vec<PresentationProvenanceItem>,
    pub template_identity: PresentationTemplateView,
    pub verification: PresentationVerificationRecord,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationRevisionSummary {
    pub revision: u32,
    pub created_at_ms: i64,
    pub scope: PresentationRevisionScope,
    pub change_summary: String,
    pub structurally_verified: bool,
    pub visually_verified: bool,
    pub exportable: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationFilmstripItem {
    pub slide_id: String,
    pub position: usize,
    pub title: String,
    pub layout_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<PresentationThumbnail>,
    pub issue_count: usize,
    pub blocker_count: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PresentationThumbnail {
    pub media_type: String,
    pub bytes_base64: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationNotesItem {
    pub slide_id: String,
    pub speaker_notes: String,
    pub source_refs: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationCitationItem {
    pub citation_id: String,
    pub slide_id: String,
    pub object_id: Option<String>,
    pub source_ref: String,
    pub evidence_ref: String,
    pub label: String,
    pub locator: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationProvenanceItem {
    pub slide_id: String,
    pub object_id: String,
    pub source_ref: String,
    pub evidence_ref: String,
    pub evidence_class: EvidenceClass,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationTemplateView {
    pub template_id: Option<String>,
    pub name: String,
    pub imported: bool,
    pub fingerprint_sha256: String,
    pub master_ids: Vec<String>,
    pub layout_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationPreviewResponse {
    pub presentation_id: String,
    pub revision: u32,
    pub filmstrip: Vec<PresentationFilmstripItem>,
    pub issues: Vec<PresentationReviewIssue>,
    pub renderer_unavailable: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationCommandError {
    pub code: String,
    pub message: String,
}

impl PresentationCommandError {
    pub(crate) fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StoredPresentationPreview {
    pub slide_id: String,
    pub path: String,
    pub media_type: String,
    pub width: u32,
    pub height: u32,
    pub sha256: String,
}
