use super::{
    exact_package_preview::qualify_exact_package, ooxml::extract_embedded_ir,
    policy::enforce_safe_package, preview::render_previews, style_xml::xml_text, zip::read_zip,
    CellValue, FormulaResult, RecalculationStatus, VerificationCheck, WorkbookLocation,
    WorkbookPreviewImage, WorkbookStatusCode, WorkbookWarning, WorkbookWarningCode,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkbookVerification {
    pub status_code: WorkbookStatusCode,
    pub structurally_verified: bool,
    pub semantically_verified: bool,
    pub visually_verified: bool,
    pub active_content_blocked: bool,
    pub external_connections_blocked: bool,
    pub formulas_verified: bool,
    pub charts_verified: bool,
    pub sheet_bounds_verified: bool,
    pub exportable: bool,
    #[serde(default)]
    pub renderer: Option<String>,
    #[serde(default)]
    pub exact_package_page_count: usize,
    pub warnings: Vec<WorkbookWarning>,
    pub previews: Vec<super::WorkbookPreviewEvidence>,
    pub evidence: Vec<VerificationCheck>,
}

pub fn verify_workbook_bytes(bytes: &[u8]) -> Result<WorkbookVerification, String> {
    verify_and_render(bytes).map(|(verification, _)| verification)
}

pub(crate) fn verify_and_render(
    bytes: &[u8],
) -> Result<(WorkbookVerification, Vec<WorkbookPreviewImage>), String> {
    let entries = read_zip(bytes)?;
    enforce_safe_package(&entries)?;
    require_parts(&entries)?;
    verify_relationship_targets(&entries)?;
    let workbook = extract_embedded_ir(&entries)?;
    verify_workbook_projection(&entries, &workbook)?;
    let (previews, mut warnings) = render_previews(&workbook)?;
    let semantic_preview_complete = previews.len() == workbook.worksheets.len();
    let cell_index = super::sheet_xml::workbook_cell_index(&workbook)?;
    warnings.extend(chart_data_warnings(&workbook, &cell_index)?);
    for sheet in &workbook.worksheets {
        for cell in &sheet.cells {
            if let CellValue::Formula {
                cached_value: Some(FormulaResult::Error { .. }),
                ..
            } = &cell.value
            {
                warnings.push(WorkbookWarning {
                    code: WorkbookWarningCode::FormulaError,
                    location: WorkbookLocation {
                        sheet_id: Some(sheet.sheet_id.clone()),
                        range: Some(cell.address.clone()),
                        chart_id: None,
                    },
                    technical_detail: "Formula has an explicit cached Excel error result."
                        .to_string(),
                });
            }
        }
    }
    let has_formulas = workbook
        .worksheets
        .iter()
        .flat_map(|sheet| &sheet.cells)
        .any(|cell| matches!(cell.value, CellValue::Formula { .. }));
    if has_formulas && workbook.recalculation.status != RecalculationStatus::Recalculated {
        warnings.push(WorkbookWarning {
            code: WorkbookWarningCode::NeedsRecalculation,
            location: WorkbookLocation::default(),
            technical_detail: "Formula results are intentionally marked stale and must be recalculated before export.".to_string(),
        });
    }
    let charts_verified = !warnings
        .iter()
        .any(|warning| matches!(warning.code, WorkbookWarningCode::ChartDataMissing));
    let (renderer, exact_package_page_count, exact_package_check) =
        match qualify_exact_package(bytes) {
            Ok(qualification) => (
                Some(qualification.renderer_identity),
                qualification.page_count,
                qualification.check,
            ),
            Err(error) => {
                if let Some(warning) = warnings
                    .iter_mut()
                    .find(|warning| warning.code == WorkbookWarningCode::PreviewUnavailable)
                {
                    warning.technical_detail = format!(
                        "{} Exact emitted-XLSX qualification also failed: {error}",
                        warning.technical_detail
                    );
                } else {
                    warnings.push(WorkbookWarning {
                        code: WorkbookWarningCode::PreviewUnavailable,
                        location: WorkbookLocation::default(),
                        technical_detail: format!(
                            "The emitted XLSX could not be visually qualified: {error}"
                        ),
                    });
                }
                (None, 0, check("exact_package_pages_rendered", false, error))
            }
        };
    let visually_verified =
        renderer.is_some() && exact_package_check.passed && visual_warnings_clear(&warnings);
    let formulas_verified =
        !has_formulas || workbook.recalculation.status == RecalculationStatus::Recalculated;
    let exportable = visually_verified
        && formulas_verified
        && charts_verified
        && !warnings
            .iter()
            .any(|warning| matches!(warning.code, WorkbookWarningCode::FormulaError));
    let status_code = if exportable {
        WorkbookStatusCode::Ready
    } else if has_formulas && !formulas_verified {
        WorkbookStatusCode::NeedsRecalculation
    } else {
        WorkbookStatusCode::CheckRequired
    };
    let preview_evidence = previews
        .iter()
        .map(|preview| preview.evidence.clone())
        .collect::<Vec<_>>();
    let evidence = vec![
        check("package_structure_valid", true, format!("{} package parts passed ZIP, CRC, content-type, and relationship checks.", entries.len())),
        check("workbook_projection_matches", true, format!("{} worksheet projections match the embedded typed contract.", workbook.worksheets.len())),
        check("active_content_absent", true, "No macro, embedded object, ActiveX, or executable relationship parts were found.".to_string()),
        check("external_connections_absent", true, "No external links, connections, query tables, or external relationship targets were found.".to_string()),
        check("semantic_sheet_images_rendered", semantic_preview_complete, format!("{} of {} per-sheet semantic images are available for UI orientation only; they do not authorize export.", previews.len(), workbook.worksheets.len())),
        exact_package_check,
        check("formula_results_current", formulas_verified, if formulas_verified { "Formula values are absent or have a qualified recalculation receipt." } else { "Formula values are marked as needing recalculation." }.to_string()),
    ];
    Ok((
        WorkbookVerification {
            status_code,
            structurally_verified: true,
            semantically_verified: true,
            visually_verified,
            active_content_blocked: true,
            external_connections_blocked: true,
            formulas_verified,
            charts_verified,
            sheet_bounds_verified: true,
            exportable,
            renderer,
            exact_package_page_count,
            warnings,
            previews: preview_evidence,
            evidence,
        },
        previews,
    ))
}

fn visual_warnings_clear(warnings: &[WorkbookWarning]) -> bool {
    !warnings.iter().any(|warning| {
        matches!(
            warning.code,
            WorkbookWarningCode::ColumnContentClipped
                | WorkbookWarningCode::ChartDataMissing
                | WorkbookWarningCode::CriticalSheetHidden
        )
    })
}

fn require_parts(entries: &BTreeMap<String, Vec<u8>>) -> Result<(), String> {
    for required in [
        "[Content_Types].xml",
        "_rels/.rels",
        "xl/workbook.xml",
        "xl/_rels/workbook.xml.rels",
        "xl/styles.xml",
        "customXml/item1.xml",
    ] {
        if !entries.contains_key(required) {
            return Err(format!(
                "Workbook package is missing required part {required}."
            ));
        }
    }
    Ok(())
}

fn verify_workbook_projection(
    entries: &BTreeMap<String, Vec<u8>>,
    workbook: &super::WorkbookIr,
) -> Result<(), String> {
    let canonical = super::ooxml::package_entries(workbook)?;
    if entries.len() != canonical.len() {
        return Err(
            "Workbook package contains parts outside its canonical typed projection.".to_string(),
        );
    }
    for (name, expected) in &canonical {
        let actual = entries
            .get(name)
            .ok_or_else(|| format!("Canonical workbook part {name} is missing."))?;
        if actual != expected {
            return Err(format!(
                "Workbook part {name} does not match its canonical typed projection."
            ));
        }
    }
    let workbook_xml = xml(entries, "xl/workbook.xml")?;
    for (index, sheet) in workbook.worksheets.iter().enumerate() {
        if !workbook_xml.contains(&format!(
            "name=\"{}\"",
            super::style_xml::xml_attr(&sheet.name)
        )) {
            return Err(format!("Workbook XML is missing worksheet {}.", sheet.name));
        }
        let path = format!("xl/worksheets/sheet{}.xml", index + 1);
        let sheet_xml = xml(entries, &path)?;
        for cell in &sheet.cells {
            if !sheet_xml.contains(&format!(
                " r=\"{}\"",
                super::style_xml::xml_attr(&cell.address)
            )) {
                return Err(format!(
                    "Worksheet {} is missing cell {}.",
                    sheet.name, cell.address
                ));
            }
            if let CellValue::Formula { expression, .. } = &cell.value {
                let formula = xml_text(expression.strip_prefix('=').unwrap_or(expression));
                if !sheet_xml.contains(&format!("<f>{formula}</f>")) {
                    return Err(format!(
                        "Worksheet {} formula at {} does not match its typed contract.",
                        sheet.name, cell.address
                    ));
                }
            }
        }
        for table in &sheet.tables {
            if !entries
                .values()
                .filter_map(|value| std::str::from_utf8(value).ok())
                .any(|value| {
                    value.contains(&format!(
                        "displayName=\"{}\"",
                        super::style_xml::xml_attr(&table.name)
                    ))
                })
            {
                return Err(format!(
                    "Workbook table {} is missing from the package.",
                    table.name
                ));
            }
        }
        for chart in &sheet.charts {
            if !entries
                .iter()
                .filter(|(name, _)| name.starts_with("xl/charts/chart"))
                .filter_map(|(_, value)| std::str::from_utf8(value).ok())
                .any(|value| value.contains(&format!("<a:t>{}</a:t>", xml_text(&chart.title))))
            {
                return Err(format!(
                    "Workbook chart {} is missing from the package.",
                    chart.chart_id
                ));
            }
        }
    }
    let expected_charts = workbook
        .worksheets
        .iter()
        .map(|sheet| sheet.charts.len())
        .sum::<usize>();
    let actual_charts = entries
        .keys()
        .filter(|name| name.starts_with("xl/charts/chart") && name.ends_with(".xml"))
        .count();
    if expected_charts != actual_charts {
        return Err("Workbook chart part count does not match its typed contract.".to_string());
    }
    Ok(())
}

fn chart_data_warnings(
    workbook: &super::WorkbookIr,
    cell_index: &super::sheet_xml::WorkbookCellIndex<'_>,
) -> Result<Vec<WorkbookWarning>, String> {
    let mut warnings = Vec::new();
    for owner in &workbook.worksheets {
        for chart in &owner.charts {
            let (category_sheet_name, category_range) =
                super::address::split_qualified_range(&chart.category_range, &owner.name)?;
            let category_sheet = workbook
                .worksheets
                .iter()
                .find(|sheet| sheet.name.eq_ignore_ascii_case(&category_sheet_name))
                .ok_or_else(|| {
                    format!(
                        "Chart {} references missing sheet {category_sheet_name}.",
                        chart.chart_id
                    )
                })?;
            if !chart_range_is_renderable(
                workbook,
                category_sheet,
                category_range,
                false,
                cell_index,
            ) {
                warnings.push(WorkbookWarning {
                    code: WorkbookWarningCode::ChartDataMissing,
                    location: WorkbookLocation {
                        sheet_id: Some(owner.sheet_id.clone()),
                        range: Some(chart.category_range.clone()),
                        chart_id: Some(chart.chart_id.clone()),
                    },
                    technical_detail:
                        "Chart categories contain a missing, blank, error, or stale source cell."
                            .to_string(),
                });
            }
            for series in &chart.series {
                let (sheet_name, range) =
                    super::address::split_qualified_range(&series.value_range, &owner.name)?;
                let sheet = workbook
                    .worksheets
                    .iter()
                    .find(|sheet| sheet.name.eq_ignore_ascii_case(&sheet_name))
                    .ok_or_else(|| {
                        format!(
                            "Chart {} references missing sheet {sheet_name}.",
                            chart.chart_id
                        )
                    })?;
                if !chart_range_is_renderable(workbook, sheet, range, true, cell_index) {
                    warnings.push(WorkbookWarning { code: WorkbookWarningCode::ChartDataMissing, location: WorkbookLocation { sheet_id: Some(owner.sheet_id.clone()), range: Some(series.value_range.clone()), chart_id: Some(chart.chart_id.clone()) }, technical_detail: "Chart series contains a missing, non-numeric, error, or stale source cell.".to_string() });
                }
            }
        }
    }
    Ok(warnings)
}

fn chart_range_is_renderable(
    workbook: &super::WorkbookIr,
    sheet: &super::Worksheet,
    range: super::address::CellRange,
    numeric: bool,
    cell_index: &super::sheet_xml::WorkbookCellIndex<'_>,
) -> bool {
    for row in range.start.row..=range.end.row {
        for column in range.start.column..=range.end.column {
            let Some(value) = cell_index
                .get(&(sheet.name.to_lowercase(), row, column))
                .copied()
            else {
                return false;
            };
            let renderable = if numeric {
                matches!(value, CellValue::Number { .. })
                    || matches!(value, CellValue::Formula { cached_value: Some(FormulaResult::Number { .. }), .. } if workbook.recalculation.status == RecalculationStatus::Recalculated)
            } else {
                matches!(
                    value,
                    CellValue::Text { .. }
                        | CellValue::Number { .. }
                        | CellValue::Boolean { .. }
                        | CellValue::Date { .. }
                ) || matches!(value, CellValue::Formula { cached_value: Some(FormulaResult::Number { .. } | FormulaResult::Text { .. } | FormulaResult::Boolean { .. }), .. } if workbook.recalculation.status == RecalculationStatus::Recalculated)
            };
            if !renderable {
                return false;
            }
        }
    }
    true
}

pub(crate) fn verify_relationship_targets(
    entries: &BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    let target =
        Regex::new(r#"Target\s*=\s*[\"']([^\"']+)[\"']"#).map_err(|error| error.to_string())?;
    for (name, bytes) in entries.iter().filter(|(name, _)| name.ends_with(".rels")) {
        let xml = std::str::from_utf8(bytes)
            .map_err(|_| format!("Relationship part {name} is not UTF-8."))?;
        let base = relationship_base(name)?;
        for capture in target.captures_iter(xml) {
            let raw = capture.get(1).unwrap().as_str();
            let resolved = resolve_target(&base, raw)?;
            if !entries.contains_key(&resolved) {
                return Err(format!(
                    "Relationship part {name} targets missing part {resolved}."
                ));
            }
        }
    }
    Ok(())
}

fn relationship_base(name: &str) -> Result<String, String> {
    if name == "_rels/.rels" {
        return Ok(String::new());
    }
    let marker = "/_rels/";
    let index = name
        .rfind(marker)
        .ok_or_else(|| format!("Relationship part path {name} is malformed."))?;
    Ok(name[..index].to_string())
}

fn resolve_target(base: &str, target: &str) -> Result<String, String> {
    if target.starts_with('/') || target.contains('\\') {
        return Err("Workbook relationship target path is unsafe.".to_string());
    }
    let joined = if base.is_empty() {
        target.to_string()
    } else {
        format!("{base}/{target}")
    };
    let mut parts = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err("Workbook relationship escapes the package root.".to_string());
                }
            }
            value => parts.push(value),
        }
    }
    Ok(parts.join("/"))
}

fn xml<'a>(entries: &'a BTreeMap<String, Vec<u8>>, name: &str) -> Result<&'a str, String> {
    std::str::from_utf8(
        entries
            .get(name)
            .ok_or_else(|| format!("Workbook part {name} is missing."))?,
    )
    .map_err(|_| format!("Workbook part {name} is not UTF-8 XML."))
}

fn check(code: &str, passed: bool, evidence: String) -> VerificationCheck {
    VerificationCheck {
        code: code.to_string(),
        passed,
        evidence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifacts::workbooks::{build_workbook, deterministic_fixture};

    #[test]
    fn detects_a_broken_internal_relationship() {
        let output = build_workbook(&deterministic_fixture().unwrap()).unwrap();
        let mut entries = read_zip(&output.bytes).unwrap();
        entries.remove("xl/styles.xml");
        let corrupted = super::super::zip::write_store_zip(&entries).unwrap();
        assert!(verify_workbook_bytes(&corrupted)
            .unwrap_err()
            .contains("styles.xml"));
    }

    #[test]
    fn chart_checks_only_its_ranges_and_checks_categories() {
        let mut unrelated = deterministic_fixture().unwrap();
        unrelated.worksheets[0]
            .cells
            .push(super::super::WorkbookCell {
                address: "E20".into(),
                value: CellValue::Formula {
                    expression: "UNSUPPORTED(B2)".into(),
                    cached_value: None,
                },
                format_id: None,
                comment: None,
                provenance: vec![],
            });
        unrelated.recalculation = super::super::RecalculationState {
            status: RecalculationStatus::Stale,
            ..super::super::RecalculationState::default()
        };
        let output = build_workbook(&unrelated).unwrap();
        assert!(!output
            .verification
            .warnings
            .iter()
            .any(|warning| warning.code == WorkbookWarningCode::ChartDataMissing));

        let mut missing_category = deterministic_fixture().unwrap();
        missing_category.worksheets[0]
            .cells
            .retain(|cell| cell.address != "A2");
        let output = build_workbook(&missing_category).unwrap();
        assert!(!output.verification.charts_verified);
        assert!(output
            .verification
            .warnings
            .iter()
            .any(
                |warning| warning.code == WorkbookWarningCode::ChartDataMissing
                    && warning.location.range.as_deref() == Some("A2:A4")
            ));
    }

    #[test]
    fn native_renderer_failures_are_orientation_warnings_not_visual_authority() {
        let workbook = deterministic_fixture().unwrap();
        for failure in ["renderer missing", "renderer crashed", "renderer timed out"] {
            let (previews, warnings) = super::super::preview::render_previews_with_native_result(
                &workbook,
                Err(failure.to_string()),
            )
            .unwrap();
            assert!(!previews.is_empty());
            assert!(warnings
                .iter()
                .any(|warning| warning.code == WorkbookWarningCode::PreviewUnavailable));
            assert!(visual_warnings_clear(&warnings));
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn emitted_xlsx_uses_the_installed_qualified_visual_chain() {
        let output = build_workbook(&deterministic_fixture().unwrap()).unwrap();
        assert!(
            output.verification.visually_verified,
            "verification: {:#?}",
            output.verification
        );
        assert!(
            output.verification.exportable,
            "verification: {:#?}",
            output.verification
        );
        assert!(output.verification.exact_package_page_count > 0);
        assert!(output.verification.renderer.as_deref().is_some_and(
            |renderer| renderer.contains(crate::artifacts::ARTIFACT_RENDERER_IDENTITY)
        ));
        assert!(output.verification.evidence.iter().any(|check| {
            check.code == "exact_package_pages_rendered"
                && check.passed
                && check.evidence.contains(&output.sha256)
        }));
    }
}
