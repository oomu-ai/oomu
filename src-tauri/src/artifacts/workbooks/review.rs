use super::{VerificationCheck, WorkbookCell, WorkbookIr, WorkbookStatusCode, WorkbookWarning};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateWorkbookRequest {
    pub project_id: String,
    pub task_id: String,
    pub task_run_id: String,
    pub workbook: WorkbookIr,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkbookListRequest {
    #[serde(default)]
    pub project_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkbookIdRequest {
    pub artifact_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkbookPreviewRequest {
    pub artifact_id: String,
    pub revision: u32,
    pub sheet_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviseWorkbookRangeRequest {
    pub artifact_id: String,
    pub base_revision: u32,
    pub sheet_id: String,
    #[serde(default)]
    pub target_range: Option<String>,
    pub instruction: String,
    #[serde(default)]
    pub replacement_cells: Option<Vec<WorkbookCell>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportWorkbookRevisionRequest {
    pub artifact_id: String,
    pub revision: u32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InspectWorkbookTemplateRequest {
    pub project_id: String,
    pub task_id: String,
    pub task_run_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateWorkbookFromTemplateRequest {
    pub project_id: String,
    pub task_id: String,
    pub task_run_id: String,
    pub template_token: String,
    pub title: String,
    pub locale: String,
    pub sheet_name: String,
    #[serde(default)]
    pub target_range: Option<String>,
    pub instruction: String,
    pub replacement_cells: Vec<WorkbookCell>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkbookReviewSummary {
    pub artifact_id: String,
    pub project_id: String,
    pub task_id: String,
    pub task_run_id: String,
    pub title: String,
    pub current_revision: u32,
    pub status_code: WorkbookStatusCode,
    pub preview_available: bool,
    pub safe_prior_revision: Option<u32>,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkbookReviewRecord {
    pub artifact_id: String,
    pub project_id: String,
    pub task_id: String,
    pub task_run_id: String,
    pub title: String,
    pub current_revision: u32,
    pub selected_sheet_id: Option<String>,
    pub preview_available: bool,
    pub safe_prior_revision: Option<u32>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub revisions: Vec<WorkbookRevisionView>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkbookRevisionView {
    pub revision: u32,
    pub status_code: WorkbookStatusCode,
    pub created_at_ms: i64,
    pub completed_at_ms: Option<i64>,
    pub sheets: Vec<WorkbookSheetView>,
    pub formula_cells: Vec<WorkbookFormulaCellView>,
    pub lineage: Vec<WorkbookLineageView>,
    pub warnings: Vec<WorkbookWarning>,
    pub numbers_status_code: WorkbookNumbersStatusCode,
    pub exportable: bool,
    pub evidence_summary: Vec<VerificationCheck>,
    pub technical_evidence_available: bool,
    pub recoverable: bool,
    pub last_error_code: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkbookSheetView {
    pub sheet_id: String,
    pub name: String,
    pub preview_available: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkbookFormulaCellView {
    pub sheet_id: String,
    pub address: String,
    pub expression: String,
    pub display_value: String,
    pub status_code: WorkbookFormulaStatusCode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkbookFormulaStatusCode {
    UpToDate,
    NeedsRecalculation,
    Error,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkbookLineageView {
    pub sheet_id: String,
    pub address: String,
    pub source_ref: String,
    pub evidence_ref: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkbookNumbersStatusCode {
    UpToDate,
    NeedsRecalculation,
    NotApplicable,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkbookPreviewResponse {
    pub artifact_id: String,
    pub revision: u32,
    pub sheet_id: String,
    pub mime_type: String,
    pub data_url: String,
    pub width: u32,
    pub height: u32,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportWorkbookRevisionResult {
    pub artifact_id: String,
    pub revision: u32,
    pub path: String,
    pub sha256: String,
    pub receipt_id: String,
    pub accounting_status_code: WorkbookExportAccountingStatusCode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkbookExportAccountingStatusCode {
    Recorded,
    RecordingPending,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkbookTemplateInspection {
    pub template_token: String,
    pub task_run_id: String,
    pub source_name: String,
    pub source_sha256: String,
    pub sheets: Vec<WorkbookTemplateSheet>,
    pub preview_qualified: bool,
    pub expires_at_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkbookTemplateSheet {
    pub sheet_id: String,
    pub name: String,
    pub row_count: u32,
    pub column_count: u32,
    pub contains_formulas: bool,
    pub visibility: super::SheetVisibility,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkbookCommandError {
    pub code: String,
    pub message: String,
}

impl WorkbookCommandError {
    pub(crate) fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredWorkbookPreview {
    pub sheet_id: String,
    pub path: String,
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
    pub sha256: String,
}
