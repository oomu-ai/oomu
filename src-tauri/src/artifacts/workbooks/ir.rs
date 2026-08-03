use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkbookIr {
    pub schema_version: u16,
    pub title: String,
    pub locale: String,
    pub date_system: WorkbookDateSystem,
    pub revision: u32,
    #[serde(default)]
    pub formats: Vec<CellFormat>,
    pub worksheets: Vec<Worksheet>,
    #[serde(default)]
    pub named_ranges: Vec<NamedRange>,
    #[serde(default)]
    pub recalculation: RecalculationState,
    #[serde(default)]
    pub policy: WorkbookPolicy,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum WorkbookDateSystem {
    #[serde(rename = "1900")]
    Excel1900,
    #[serde(rename = "1904")]
    Excel1904,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Worksheet {
    pub sheet_id: String,
    pub name: String,
    pub bounds: WorksheetBounds,
    #[serde(default)]
    pub visibility: SheetVisibility,
    #[serde(default)]
    pub critical: bool,
    #[serde(default)]
    pub cells: Vec<WorkbookCell>,
    #[serde(default)]
    pub merged_ranges: Vec<String>,
    #[serde(default)]
    pub column_widths: Vec<ColumnWidth>,
    #[serde(default)]
    pub tables: Vec<WorkbookTable>,
    #[serde(default)]
    pub validations: Vec<DataValidation>,
    #[serde(default)]
    pub charts: Vec<WorkbookChart>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorksheetBounds {
    pub row_count: u32,
    pub column_count: u32,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SheetVisibility {
    #[default]
    Visible,
    Hidden,
    VeryHidden,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkbookCell {
    pub address: String,
    pub value: CellValue,
    #[serde(default)]
    pub format_id: Option<String>,
    #[serde(default)]
    pub comment: Option<CellComment>,
    #[serde(default)]
    pub provenance: Vec<ProvenanceReference>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CellValue {
    Blank,
    Text {
        value: String,
    },
    Number {
        value: f64,
    },
    Boolean {
        value: bool,
    },
    Date {
        iso: String,
    },
    Formula {
        expression: String,
        #[serde(default)]
        cached_value: Option<FormulaResult>,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum FormulaResult {
    Number { value: f64 },
    Text { value: String },
    Boolean { value: bool },
    Error { code: String },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CellComment {
    pub author: String,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProvenanceReference {
    pub source_ref: String,
    pub evidence_ref: String,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CellFormat {
    pub format_id: String,
    #[serde(default)]
    pub font: FontStyle,
    #[serde(default)]
    pub fill_color: Option<String>,
    #[serde(default)]
    pub number_format: Option<String>,
    #[serde(default)]
    pub alignment: CellAlignment,
    #[serde(default)]
    pub wrap_text: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FontStyle {
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub size_pt: Option<f32>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CellAlignment {
    #[default]
    General,
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ColumnWidth {
    pub column: String,
    pub width: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkbookTable {
    pub table_id: String,
    pub name: String,
    pub range: String,
    pub columns: Vec<String>,
    #[serde(default = "default_table_style")]
    pub style: String,
}

fn default_table_style() -> String {
    "TableStyleMedium2".to_string()
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataValidation {
    pub validation_id: String,
    pub range: String,
    pub rule: ValidationRule,
    #[serde(default)]
    pub allow_blank: bool,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ValidationRule {
    List {
        values: Vec<String>,
    },
    WholeNumber {
        minimum: i64,
        maximum: i64,
    },
    Decimal {
        minimum: f64,
        maximum: f64,
    },
    Date {
        minimum_iso: String,
        maximum_iso: String,
    },
    CustomFormula {
        formula: String,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkbookChart {
    pub chart_id: String,
    pub kind: ChartKind,
    pub title: String,
    pub category_range: String,
    pub series: Vec<ChartSeries>,
    pub anchor: ChartAnchor,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartKind {
    Bar,
    Column,
    Line,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChartSeries {
    pub name: String,
    pub value_range: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChartAnchor {
    pub from_column: u32,
    pub from_row: u32,
    pub to_column: u32,
    pub to_row: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NamedRange {
    pub name: String,
    pub formula: String,
    #[serde(default)]
    pub comment: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecalculationStatus {
    #[default]
    NotRequired,
    Stale,
    Recalculated,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecalculationState {
    pub status: RecalculationStatus,
    #[serde(default)]
    pub engine: Option<String>,
    #[serde(default)]
    pub engine_version: Option<String>,
    #[serde(default)]
    pub qualified: bool,
    #[serde(default)]
    pub recalculated_at_ms: Option<i64>,
    #[serde(default)]
    pub input_digest: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentPolicy {
    Forbid,
}

impl Default for ContentPolicy {
    fn default() -> Self {
        Self::Forbid
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkbookPolicy {
    #[serde(default)]
    pub macros: ContentPolicy,
    #[serde(default)]
    pub external_links: ContentPolicy,
    #[serde(default)]
    pub external_data_connections: ContentPolicy,
    #[serde(default)]
    pub hidden_executable_content: ContentPolicy,
    #[serde(default)]
    pub hidden_critical_sheets: ContentPolicy,
}
