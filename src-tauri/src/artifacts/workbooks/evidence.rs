use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkbookStatusCode {
    Building,
    Ready,
    NeedsRecalculation,
    CheckRequired,
    Failed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkbookWarningCode {
    ColumnContentClipped,
    PreviewTruncated,
    ChartDataMissing,
    FormulaError,
    CriticalSheetHidden,
    NeedsRecalculation,
    PreviewUnavailable,
    PreviewUnsupportedCharacters,
    PackageRelationshipInvalid,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkbookLocation {
    #[serde(default)]
    pub sheet_id: Option<String>,
    #[serde(default)]
    pub range: Option<String>,
    #[serde(default)]
    pub chart_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkbookWarning {
    pub code: WorkbookWarningCode,
    pub location: WorkbookLocation,
    pub technical_detail: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkbookPreviewEvidence {
    pub sheet_id: String,
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
    pub sha256: String,
}

#[derive(Clone, Debug)]
pub struct WorkbookPreviewImage {
    pub evidence: WorkbookPreviewEvidence,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VerificationCheck {
    pub code: String,
    pub passed: bool,
    pub evidence: String,
}
