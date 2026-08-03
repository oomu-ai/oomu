use super::*;
use std::{fs, io::Write, path::Path};

pub fn deterministic_fixture() -> Result<WorkbookIr, String> {
    let provenance = vec![ProvenanceReference {
        source_ref: "quarterly-ledger".to_string(),
        evidence_ref: "evidence-quarterly-ledger-sha256".to_string(),
        note: Some("Bounded fixture source".to_string()),
    }];
    let formats = vec![
        CellFormat {
            format_id: "header".into(),
            font: FontStyle {
                bold: true,
                italic: false,
                color: Some("FFFFFF".into()),
                size_pt: Some(11.0),
            },
            fill_color: Some("1D4ED8".into()),
            number_format: None,
            alignment: CellAlignment::Center,
            wrap_text: false,
        },
        CellFormat {
            format_id: "currency".into(),
            font: FontStyle::default(),
            fill_color: None,
            number_format: Some("$#,##0.00".into()),
            alignment: CellAlignment::Right,
            wrap_text: false,
        },
        CellFormat {
            format_id: "total".into(),
            font: FontStyle {
                bold: true,
                italic: false,
                color: Some("111827".into()),
                size_pt: Some(11.0),
            },
            fill_color: Some("DBEAFE".into()),
            number_format: Some("$#,##0.00".into()),
            alignment: CellAlignment::Right,
            wrap_text: false,
        },
        CellFormat {
            format_id: "note".into(),
            font: FontStyle {
                bold: false,
                italic: true,
                color: Some("374151".into()),
                size_pt: Some(10.0),
            },
            fill_color: Some("F3F4F6".into()),
            number_format: None,
            alignment: CellAlignment::Left,
            wrap_text: true,
        },
    ];
    let sales = Worksheet {
        sheet_id: "quarterly_sales".into(),
        name: "Quarterly Sales".into(),
        bounds: WorksheetBounds {
            row_count: 24,
            column_count: 12,
        },
        visibility: SheetVisibility::Visible,
        critical: true,
        cells: vec![
            text("A1", "Region", Some("header")),
            text("B1", "Revenue", Some("header")),
            text("C1", "Closed", Some("header")),
            text("D1", "Status", Some("header")),
            sourced_text("A2", "North", provenance.clone()),
            number("B2", 1_200.0, Some("currency")),
            date("C2", "2026-04-30"),
            text("D2", "Reviewed", None),
            sourced_text("A3", "South", provenance.clone()),
            number("B3", 1_450.0, Some("currency")),
            date("C3", "2026-05-31"),
            text("D3", "Reviewed", None),
            sourced_text("A4", "West", provenance.clone()),
            number("B4", 1_250.0, Some("currency")),
            date("C4", "2026-06-30"),
            text("D4", "Pending", None),
            text("A5", "Total", Some("total")),
            WorkbookCell {
                address: "B5".into(),
                value: CellValue::Formula {
                    expression: "SUM(B2:B4)".into(),
                    cached_value: None,
                },
                format_id: Some("total".into()),
                comment: Some(CellComment {
                    author: "OOMU".into(),
                    text: "Calculated from the three visible regional values.".into(),
                }),
                provenance: provenance.clone(),
            },
        ],
        merged_ranges: vec![],
        column_widths: vec![
            ColumnWidth {
                column: "A".into(),
                width: 18.0,
            },
            ColumnWidth {
                column: "B".into(),
                width: 16.0,
            },
            ColumnWidth {
                column: "C".into(),
                width: 15.0,
            },
            ColumnWidth {
                column: "D".into(),
                width: 16.0,
            },
        ],
        tables: vec![WorkbookTable {
            table_id: "sales_table".into(),
            name: "QuarterlySalesTable".into(),
            range: "A1:D4".into(),
            columns: vec![
                "Region".into(),
                "Revenue".into(),
                "Closed".into(),
                "Status".into(),
            ],
            style: "TableStyleMedium2".into(),
        }],
        validations: vec![DataValidation {
            validation_id: "sales_status".into(),
            range: "D2:D4".into(),
            rule: ValidationRule::List {
                values: vec!["Reviewed".into(), "Pending".into()],
            },
            allow_blank: false,
            prompt: Some("Choose a review state".into()),
            error: Some("Choose a listed state".into()),
        }],
        charts: vec![WorkbookChart {
            chart_id: "revenue_by_region".into(),
            kind: ChartKind::Column,
            title: "Revenue by region".into(),
            category_range: "A2:A4".into(),
            series: vec![ChartSeries {
                name: "Revenue".into(),
                value_range: "B2:B4".into(),
            }],
            anchor: ChartAnchor {
                from_column: 4,
                from_row: 1,
                to_column: 11,
                to_row: 14,
            },
        }],
    };
    let sources = Worksheet {
        sheet_id: "source_notes".into(),
        name: "Source Notes".into(),
        bounds: WorksheetBounds {
            row_count: 12,
            column_count: 6,
        },
        visibility: SheetVisibility::Visible,
        critical: false,
        cells: vec![
            text("A1", "Source", Some("header")),
            text("B1", "Evidence", Some("header")),
            text("C1", "Checked", Some("header")),
            sourced_text("A2", "Quarterly ledger", provenance.clone()),
            text("B2", "Bound to evidence digest", Some("note")),
            date("C2", "2026-07-11T16:00:00Z"),
        ],
        merged_ranges: vec![],
        column_widths: vec![
            ColumnWidth {
                column: "A".into(),
                width: 24.0,
            },
            ColumnWidth {
                column: "B".into(),
                width: 34.0,
            },
            ColumnWidth {
                column: "C".into(),
                width: 22.0,
            },
        ],
        tables: vec![WorkbookTable {
            table_id: "sources_table".into(),
            name: "WorkbookSourcesTable".into(),
            range: "A1:C2".into(),
            columns: vec!["Source".into(), "Evidence".into(), "Checked".into()],
            style: "TableStyleMedium4".into(),
        }],
        validations: vec![],
        charts: vec![],
    };
    let mut workbook = WorkbookIr {
        schema_version: WORKBOOK_IR_SCHEMA_VERSION,
        title: "Verified Quarterly Sales".into(),
        locale: "en-US".into(),
        date_system: WorkbookDateSystem::Excel1900,
        revision: 1,
        formats,
        worksheets: vec![sales, sources],
        named_ranges: vec![NamedRange {
            name: "TotalRevenue".into(),
            formula: "'Quarterly Sales'!$B$5".into(),
            comment: Some("Qualified total".into()),
        }],
        recalculation: RecalculationState {
            status: RecalculationStatus::Stale,
            ..RecalculationState::default()
        },
        policy: WorkbookPolicy::default(),
    };
    recalculate_supported_formulas(&mut workbook)?;
    Ok(workbook)
}

pub fn write_deterministic_fixture(path: &Path) -> Result<WorkbookBuildOutput, String> {
    let output = build_workbook(&deterministic_fixture()?)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|error| error.to_string())?;
    file.write_all(&output.bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| error.to_string())?;
    let written =
        crate::foundation::digest::sha256_file_hex(path).map_err(|error| error.to_string())?;
    if written != output.sha256 {
        return Err("Deterministic fixture write digest mismatch.".to_string());
    }
    Ok(output)
}

fn text(address: &str, value: &str, format: Option<&str>) -> WorkbookCell {
    WorkbookCell {
        address: address.into(),
        value: CellValue::Text {
            value: value.into(),
        },
        format_id: format.map(str::to_string),
        comment: None,
        provenance: vec![],
    }
}
fn sourced_text(address: &str, value: &str, provenance: Vec<ProvenanceReference>) -> WorkbookCell {
    WorkbookCell {
        address: address.into(),
        value: CellValue::Text {
            value: value.into(),
        },
        format_id: None,
        comment: None,
        provenance,
    }
}
fn number(address: &str, value: f64, format: Option<&str>) -> WorkbookCell {
    WorkbookCell {
        address: address.into(),
        value: CellValue::Number { value },
        format_id: format.map(str::to_string),
        comment: None,
        provenance: vec![],
    }
}
fn date(address: &str, value: &str) -> WorkbookCell {
    WorkbookCell {
        address: address.into(),
        value: CellValue::Date { iso: value.into() },
        format_id: None,
        comment: None,
        provenance: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_covers_formula_table_chart_comment_provenance_dates_and_multiple_sheets() {
        let workbook = deterministic_fixture().unwrap();
        assert_eq!(workbook.worksheets.len(), 2);
        assert!(workbook.worksheets[0]
            .cells
            .iter()
            .any(|cell| cell.comment.is_some() && !cell.provenance.is_empty()));
        assert!(!workbook.worksheets[0].charts.is_empty());
        assert_eq!(
            workbook.recalculation.status,
            RecalculationStatus::Recalculated
        );
    }

    #[test]
    #[ignore = "writes only when OOMU_WORKBOOK_FIXTURE_PATH is provided for external QA"]
    fn emit_deterministic_workbook_fixture() {
        let path = std::env::var("OOMU_WORKBOOK_FIXTURE_PATH").expect("fixture output path");
        write_deterministic_fixture(Path::new(&path)).unwrap();
    }
}
