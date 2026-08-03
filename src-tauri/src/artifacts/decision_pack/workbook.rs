use super::workbook_helpers::rate_flag;
use super::workbook_source::research_source_cells;
use super::{
    evidence::{exception_evidence, margin_evidence, rate_evidence},
    validate_decision_pack_analysis, DecisionPackAnalysis,
};
use crate::artifacts::workbooks::*;

const SOURCE_SHEET: &str = "Source Data";

pub(crate) fn build_decision_workbook(
    analysis: &DecisionPackAnalysis,
) -> Result<WorkbookIr, String> {
    validate_decision_pack_analysis(analysis)?;
    let rate_source_start = 2_u32;
    let margin_source_start = rate_source_start + analysis.rate_reconciliations.len() as u32;
    let exception_source_start = margin_source_start + analysis.margin_assessments.len() as u32;
    let web_source_start = exception_source_start + analysis.exceptions.len() as u32;
    let gap_source_start = web_source_start + analysis.web_claims.len() as u32;
    let source_last_row = gap_source_start
        .saturating_add(analysis.research_gaps.len() as u32)
        .saturating_sub(1);
    let mut workbook = WorkbookIr {
        schema_version: WORKBOOK_IR_SCHEMA_VERSION,
        title: analysis.title.clone(),
        locale: "en-US".to_string(),
        date_system: WorkbookDateSystem::Excel1900,
        revision: 1,
        formats: formats(),
        worksheets: vec![
            source_sheet(
                analysis,
                rate_source_start,
                margin_source_start,
                exception_source_start,
                web_source_start,
                gap_source_start,
                source_last_row,
            ),
            rate_sheet(analysis, rate_source_start),
            margin_sheet(analysis, margin_source_start),
            exception_sheet(analysis),
            recommendation_sheet(analysis),
        ],
        named_ranges: vec![NamedRange {
            name: "DecisionRecommendation".to_string(),
            formula: "'Recommendation'!$B$3".to_string(),
            comment: Some("Canonical supplier decision recommendation".to_string()),
        }],
        recalculation: RecalculationState {
            status: RecalculationStatus::Stale,
            ..RecalculationState::default()
        },
        policy: WorkbookPolicy::default(),
    };
    recalculate_supported_formulas(&mut workbook)?;
    validate_workbook(&workbook)?;
    Ok(workbook)
}

fn source_sheet(
    analysis: &DecisionPackAnalysis,
    rate_start: u32,
    margin_start: u32,
    exception_start: u32,
    web_start: u32,
    gap_start: u32,
    last_row: u32,
) -> Worksheet {
    let headers = [
        "Type",
        "Name",
        "Primary",
        "Comparison",
        "Raw Estimated Cost",
        "COGS Allocation",
        "Status / Notes",
        "URL",
        "Accessed",
        "Research Subject",
        "Authority",
        "Authority Class",
        "Effective Date",
        "Date Evidence",
        "Evidence Digest",
    ];
    let mut cells = header_cells(&headers);
    for (index, rate) in analysis.rate_reconciliations.iter().enumerate() {
        let row = rate_start + index as u32;
        let evidence = rate_evidence(index);
        let provenance = provenance(&evidence.source_ref, &evidence.evidence_ref);
        cells.extend([
            sourced_text(row, "A", "Rate", provenance.clone(), None),
            sourced_text(row, "B", &rate.name, provenance.clone(), Some("wrap")),
            sourced_number(
                row,
                "C",
                rate.historical_rate,
                provenance.clone(),
                Some("amount"),
            ),
            sourced_number(
                row,
                "D",
                rate.active_quote,
                provenance.clone(),
                Some("amount"),
            ),
            sourced_text(row, "G", &rate.status, provenance, Some("wrap")),
        ]);
    }
    for (index, margin) in analysis.margin_assessments.iter().enumerate() {
        let row = margin_start + index as u32;
        let evidence = margin_evidence(index);
        let provenance = provenance(&evidence.source_ref, &evidence.evidence_ref);
        cells.extend([
            sourced_text(row, "A", "Margin", provenance.clone(), None),
            sourced_text(row, "B", &margin.name, provenance.clone(), Some("wrap")),
            sourced_number(
                row,
                "C",
                margin.margin_percent,
                provenance.clone(),
                Some("percent"),
            ),
            sourced_number(
                row,
                "D",
                margin.threshold_percent,
                provenance.clone(),
                Some("percent"),
            ),
            sourced_number(
                row,
                "E",
                margin.raw_estimated_cost,
                provenance.clone(),
                Some("amount"),
            ),
            sourced_number(
                row,
                "F",
                margin.cogs_allocation,
                provenance.clone(),
                Some("amount"),
            ),
            sourced_text(row, "G", &margin.notes, provenance, Some("wrap")),
        ]);
    }
    for (index, exception) in analysis.exceptions.iter().enumerate() {
        let row = exception_start + index as u32;
        let evidence = exception_evidence(index);
        let provenance = provenance(&evidence.source_ref, &evidence.evidence_ref);
        cells.extend([
            sourced_text(row, "A", "Exception", provenance.clone(), None),
            sourced_text(
                row,
                "B",
                &format!("Exception {}", index + 1),
                provenance.clone(),
                None,
            ),
            sourced_text(row, "G", exception, provenance, Some("wrap")),
        ]);
    }
    cells.extend(research_source_cells(analysis, web_start, gap_start));
    Worksheet {
        sheet_id: "source_data".to_string(),
        name: SOURCE_SHEET.to_string(),
        bounds: WorksheetBounds {
            row_count: last_row.saturating_add(2).max(20),
            column_count: 15,
        },
        visibility: SheetVisibility::Visible,
        critical: true,
        cells,
        merged_ranges: Vec::new(),
        column_widths: widths(&[
            ("A", 12.0),
            ("B", 18.0),
            ("C", 12.0),
            ("D", 12.0),
            ("E", 14.0),
            ("F", 14.0),
            ("G", 30.0),
            ("H", 30.0),
            ("I", 20.0),
            ("J", 14.0),
            ("K", 22.0),
            ("L", 18.0),
            ("M", 13.0),
            ("N", 18.0),
            ("O", 26.0),
        ]),
        tables: vec![WorkbookTable {
            table_id: "source_data_table".to_string(),
            name: "DecisionPackSourceData".to_string(),
            range: format!("A1:O{last_row}"),
            columns: headers.iter().map(|value| (*value).to_string()).collect(),
            style: "TableStyleMedium2".to_string(),
        }],
        validations: Vec::new(),
        charts: Vec::new(),
    }
}

fn rate_sheet(analysis: &DecisionPackAnalysis, source_start: u32) -> Worksheet {
    let headers = [
        "Supplier / Item",
        "Historical Rate",
        "Active Quote",
        "Variance",
        "Status",
        "Flag",
    ];
    let mut cells = header_cells(&headers);
    for (index, rate) in analysis.rate_reconciliations.iter().enumerate() {
        let row = index as u32 + 2;
        let source_row = source_start + index as u32;
        let evidence = rate_evidence(index);
        let provenance = provenance(&evidence.source_ref, &evidence.evidence_ref);
        let (flag, flag_format) = rate_flag(rate.historical_rate, rate.active_quote, &rate.status);
        cells.extend([
            sourced_text(row, "A", &rate.name, provenance.clone(), Some("wrap")),
            sourced_formula(
                row,
                "B",
                &format!("'{SOURCE_SHEET}'!C{source_row}"),
                provenance.clone(),
                Some("amount"),
            ),
            sourced_formula(
                row,
                "C",
                &format!("'{SOURCE_SHEET}'!D{source_row}"),
                provenance.clone(),
                Some("amount"),
            ),
            sourced_formula(
                row,
                "D",
                &format!("C{row}-B{row}"),
                provenance.clone(),
                Some("amount"),
            ),
            sourced_text(row, "E", &rate.status, provenance.clone(), Some("wrap")),
            sourced_text(row, "F", flag, provenance, Some(flag_format)),
        ]);
    }
    let last_row = analysis.rate_reconciliations.len() as u32 + 1;
    Worksheet {
        sheet_id: "rate_reconciliation".to_string(),
        name: "Rate Reconciliation".to_string(),
        bounds: WorksheetBounds {
            row_count: last_row.saturating_add(2).max(20),
            column_count: 12,
        },
        visibility: SheetVisibility::Visible,
        critical: true,
        cells,
        merged_ranges: Vec::new(),
        column_widths: widths(&[
            ("A", 28.0),
            ("B", 18.0),
            ("C", 18.0),
            ("D", 18.0),
            ("E", 22.0),
            ("F", 16.0),
        ]),
        tables: vec![WorkbookTable {
            table_id: "rate_reconciliation_table".to_string(),
            name: "RateReconciliation".to_string(),
            range: format!("A1:F{last_row}"),
            columns: headers.iter().map(|value| (*value).to_string()).collect(),
            style: "TableStyleMedium2".to_string(),
        }],
        validations: Vec::new(),
        charts: vec![WorkbookChart {
            chart_id: "rate_comparison".to_string(),
            kind: ChartKind::Column,
            title: "Historical rate vs active quote".to_string(),
            category_range: format!("A2:A{last_row}"),
            series: vec![
                ChartSeries {
                    name: "Historical rate".to_string(),
                    value_range: format!("B2:B{last_row}"),
                },
                ChartSeries {
                    name: "Active quote".to_string(),
                    value_range: format!("C2:C{last_row}"),
                },
            ],
            anchor: ChartAnchor {
                from_column: 0,
                from_row: last_row + 2,
                to_column: 11,
                to_row: last_row + 16,
            },
        }],
    }
}

fn margin_sheet(analysis: &DecisionPackAnalysis, source_start: u32) -> Worksheet {
    let headers = [
        "Supplier",
        "Raw Estimated Cost",
        "COGS Allocation",
        "Gross Profit",
        "Calculated Margin Ratio",
        "Calculated Margin %",
        "Reported Margin %",
        "Reconciliation Gap",
        "Threshold %",
        "Flag",
        "Notes",
    ];
    let mut cells = header_cells(&headers);
    for (index, margin) in analysis.margin_assessments.iter().enumerate() {
        let row = index as u32 + 2;
        let source_row = source_start + index as u32;
        let evidence = margin_evidence(index);
        let provenance = provenance(&evidence.source_ref, &evidence.evidence_ref);
        let meets = margin.margin_percent >= margin.threshold_percent;
        cells.extend([
            sourced_text(row, "A", &margin.name, provenance.clone(), Some("wrap")),
            sourced_formula(
                row,
                "B",
                &format!("'{SOURCE_SHEET}'!E{source_row}"),
                provenance.clone(),
                Some("amount"),
            ),
            sourced_formula(
                row,
                "C",
                &format!("'{SOURCE_SHEET}'!F{source_row}"),
                provenance.clone(),
                Some("amount"),
            ),
            sourced_formula(
                row,
                "D",
                &format!("B{row}-C{row}"),
                provenance.clone(),
                Some("amount"),
            ),
            sourced_formula(
                row,
                "E",
                &format!("D{row}/B{row}"),
                provenance.clone(),
                Some("ratio"),
            ),
            sourced_formula(
                row,
                "F",
                &format!("E{row}*100"),
                provenance.clone(),
                Some("percent"),
            ),
            sourced_formula(
                row,
                "G",
                &format!("'{SOURCE_SHEET}'!C{source_row}"),
                provenance.clone(),
                Some("percent"),
            ),
            sourced_formula(
                row,
                "H",
                &format!("G{row}-F{row}"),
                provenance.clone(),
                Some("percent"),
            ),
            sourced_formula(
                row,
                "I",
                &format!("'{SOURCE_SHEET}'!D{source_row}"),
                provenance.clone(),
                Some("percent"),
            ),
            sourced_text(
                row,
                "J",
                if margin_reconciliation_gap(margin).abs() > 0.05 {
                    "Margin mismatch"
                } else if meets {
                    "Reconciled / Meets threshold"
                } else {
                    "Reconciled / Below threshold"
                },
                provenance.clone(),
                Some(
                    if meets && margin_reconciliation_gap(margin).abs() <= 0.05 {
                        "flag_ok"
                    } else {
                        "flag_review"
                    },
                ),
            ),
            sourced_text(row, "K", &margin.notes, provenance, Some("wrap")),
        ]);
    }
    let last_row = analysis.margin_assessments.len() as u32 + 1;
    Worksheet {
        sheet_id: "margin_assessment".to_string(),
        name: "Margin Assessment".to_string(),
        bounds: WorksheetBounds {
            row_count: last_row.saturating_add(2).max(20),
            column_count: 12,
        },
        visibility: SheetVisibility::Visible,
        critical: true,
        cells,
        merged_ranges: Vec::new(),
        column_widths: widths(&[
            ("A", 28.0),
            ("B", 20.0),
            ("C", 18.0),
            ("D", 16.0),
            ("E", 19.0),
            ("F", 18.0),
            ("G", 18.0),
            ("H", 18.0),
            ("I", 15.0),
            ("J", 28.0),
            ("K", 42.0),
        ]),
        tables: vec![WorkbookTable {
            table_id: "margin_assessment_table".to_string(),
            name: "MarginAssessment".to_string(),
            range: format!("A1:K{last_row}"),
            columns: headers.iter().map(|value| (*value).to_string()).collect(),
            style: "TableStyleMedium4".to_string(),
        }],
        validations: Vec::new(),
        charts: vec![WorkbookChart {
            chart_id: "margin_comparison".to_string(),
            kind: ChartKind::Bar,
            title: "Margin vs threshold".to_string(),
            category_range: format!("A2:A{last_row}"),
            series: vec![
                ChartSeries {
                    name: "Margin".to_string(),
                    value_range: format!("G2:G{last_row}"),
                },
                ChartSeries {
                    name: "Threshold".to_string(),
                    value_range: format!("I2:I{last_row}"),
                },
            ],
            anchor: ChartAnchor {
                from_column: 0,
                from_row: last_row + 2,
                to_column: 11,
                to_row: last_row + 16,
            },
        }],
    }
}

fn margin_reconciliation_gap(margin: &super::MarginAssessment) -> f64 {
    let calculated =
        ((margin.raw_estimated_cost - margin.cogs_allocation) / margin.raw_estimated_cost) * 100.0;
    margin.margin_percent - calculated
}

fn exception_sheet(analysis: &DecisionPackAnalysis) -> Worksheet {
    let headers = ["Exception", "Review State"];
    let mut cells = header_cells(&headers);
    if analysis.exceptions.is_empty() {
        cells.extend([
            text_cell(
                2,
                "A",
                "No material exceptions identified in the canonical analysis.",
                Some("wrap"),
            ),
            text_cell(2, "B", "Clear", Some("flag_ok")),
        ]);
    } else {
        for (index, exception) in analysis.exceptions.iter().enumerate() {
            let row = index as u32 + 2;
            let evidence = exception_evidence(index);
            let provenance = provenance(&evidence.source_ref, &evidence.evidence_ref);
            cells.extend([
                sourced_text(row, "A", exception, provenance.clone(), Some("wrap")),
                sourced_text(row, "B", "Requires review", provenance, Some("flag_review")),
            ]);
        }
    }
    let last_row = analysis.exceptions.len().max(1) as u32 + 1;
    Worksheet {
        sheet_id: "exceptions".to_string(),
        name: "Exceptions".to_string(),
        bounds: WorksheetBounds {
            row_count: last_row.saturating_add(2).max(12),
            column_count: 4,
        },
        visibility: SheetVisibility::Visible,
        critical: true,
        cells,
        merged_ranges: Vec::new(),
        column_widths: widths(&[("A", 80.0), ("B", 20.0)]),
        tables: vec![WorkbookTable {
            table_id: "exceptions_table".to_string(),
            name: "DecisionExceptions".to_string(),
            range: format!("A1:B{last_row}"),
            columns: headers.iter().map(|value| (*value).to_string()).collect(),
            style: "TableStyleMedium3".to_string(),
        }],
        validations: Vec::new(),
        charts: Vec::new(),
    }
}

fn recommendation_sheet(analysis: &DecisionPackAnalysis) -> Worksheet {
    let headers = ["Decision Field", "Content"];
    Worksheet {
        sheet_id: "recommendation".to_string(),
        name: "Recommendation".to_string(),
        bounds: WorksheetBounds {
            row_count: 12,
            column_count: 4,
        },
        visibility: SheetVisibility::Visible,
        critical: true,
        cells: vec![
            text_cell(1, "A", headers[0], Some("header")),
            text_cell(1, "B", headers[1], Some("header")),
            text_cell(2, "A", "Executive summary", Some("label")),
            text_cell(2, "B", &analysis.executive_summary, Some("wrap")),
            text_cell(3, "A", "Recommendation", Some("label")),
            text_cell(3, "B", &analysis.recommendation, Some("recommendation")),
            text_cell(4, "A", "Email summary", Some("label")),
            text_cell(4, "B", &analysis.email_summary, Some("wrap")),
        ],
        merged_ranges: Vec::new(),
        column_widths: widths(&[("A", 24.0), ("B", 100.0)]),
        tables: vec![WorkbookTable {
            table_id: "recommendation_table".to_string(),
            name: "DecisionRecommendationTable".to_string(),
            range: "A1:B4".to_string(),
            columns: headers.iter().map(|value| (*value).to_string()).collect(),
            style: "TableStyleMedium2".to_string(),
        }],
        validations: Vec::new(),
        charts: Vec::new(),
    }
}

fn formats() -> Vec<CellFormat> {
    vec![
        format(
            "header",
            true,
            Some("FFFFFF"),
            Some("1F4E78"),
            None,
            CellAlignment::Center,
            true,
        ),
        format(
            "label",
            true,
            Some("1F2937"),
            Some("DCE6F1"),
            None,
            CellAlignment::Left,
            true,
        ),
        format(
            "amount",
            false,
            None,
            None,
            Some("#,##0.00"),
            CellAlignment::Right,
            false,
        ),
        format(
            "percent",
            false,
            None,
            None,
            Some("0.00\"%\""),
            CellAlignment::Right,
            false,
        ),
        format(
            "ratio",
            false,
            None,
            None,
            Some("0.0000"),
            CellAlignment::Right,
            false,
        ),
        format("wrap", false, None, None, None, CellAlignment::Left, true),
        format(
            "flag_ok",
            true,
            Some("166534"),
            Some("DCFCE7"),
            None,
            CellAlignment::Center,
            true,
        ),
        format(
            "flag_review",
            true,
            Some("991B1B"),
            Some("FEE2E2"),
            None,
            CellAlignment::Center,
            true,
        ),
        format(
            "recommendation",
            true,
            Some("FFFFFF"),
            Some("0B57D0"),
            None,
            CellAlignment::Left,
            true,
        ),
    ]
}

#[allow(clippy::too_many_arguments)]
fn format(
    id: &str,
    bold: bool,
    font_color: Option<&str>,
    fill_color: Option<&str>,
    number_format: Option<&str>,
    alignment: CellAlignment,
    wrap_text: bool,
) -> CellFormat {
    CellFormat {
        format_id: id.to_string(),
        font: FontStyle {
            bold,
            italic: false,
            color: font_color.map(str::to_string),
            size_pt: Some(10.0),
        },
        fill_color: fill_color.map(str::to_string),
        number_format: number_format.map(str::to_string),
        alignment,
        wrap_text,
    }
}

fn header_cells(headers: &[&str]) -> Vec<WorkbookCell> {
    headers
        .iter()
        .enumerate()
        .map(|(index, value)| text_cell(1, column(index), value, Some("header")))
        .collect()
}

fn column(index: usize) -> &'static str {
    const COLUMNS: [&str; 15] = [
        "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O",
    ];
    COLUMNS[index]
}

fn text_cell(row: u32, column: &str, value: &str, format_id: Option<&str>) -> WorkbookCell {
    WorkbookCell {
        address: format!("{column}{row}"),
        value: CellValue::Text {
            value: value.to_string(),
        },
        format_id: format_id.map(str::to_string),
        comment: None,
        provenance: Vec::new(),
    }
}

pub(super) fn sourced_text(
    row: u32,
    column: &str,
    value: &str,
    provenance: Vec<ProvenanceReference>,
    format_id: Option<&str>,
) -> WorkbookCell {
    let mut cell = text_cell(row, column, value, format_id);
    cell.provenance = provenance;
    cell
}

fn sourced_number(
    row: u32,
    column: &str,
    value: f64,
    provenance: Vec<ProvenanceReference>,
    format_id: Option<&str>,
) -> WorkbookCell {
    WorkbookCell {
        address: format!("{column}{row}"),
        value: CellValue::Number { value },
        format_id: format_id.map(str::to_string),
        comment: None,
        provenance,
    }
}

fn sourced_formula(
    row: u32,
    column: &str,
    expression: &str,
    provenance: Vec<ProvenanceReference>,
    format_id: Option<&str>,
) -> WorkbookCell {
    WorkbookCell {
        address: format!("{column}{row}"),
        value: CellValue::Formula {
            expression: expression.to_string(),
            cached_value: None,
        },
        format_id: format_id.map(str::to_string),
        comment: Some(CellComment {
            author: "OOMU".to_string(),
            text: "Formula derived from the canonical decision-pack analysis.".to_string(),
        }),
        provenance,
    }
}

pub(super) fn provenance(source_ref: &str, evidence_ref: &str) -> Vec<ProvenanceReference> {
    vec![ProvenanceReference {
        source_ref: source_ref.to_string(),
        evidence_ref: evidence_ref.to_string(),
        note: Some("Canonical decision-pack analysis input".to_string()),
    }]
}

fn widths(values: &[(&str, f64)]) -> Vec<ColumnWidth> {
    values
        .iter()
        .map(|(column, width)| ColumnWidth {
            column: (*column).to_string(),
            width: *width,
        })
        .collect()
}
