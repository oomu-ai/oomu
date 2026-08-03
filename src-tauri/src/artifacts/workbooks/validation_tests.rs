use super::*;
use crate::artifacts::workbooks::{
    fixture::deterministic_fixture, RecalculationState, RecalculationStatus, WorkbookCell,
    WorksheetBounds,
};

#[test]
fn fixture_is_valid_and_external_formula_is_rejected() {
    let workbook = deterministic_fixture().unwrap();
    validate_workbook(&workbook).unwrap();
    let mut active = workbook;
    active.worksheets[0].cells[0].value = CellValue::Formula {
        expression: "WEBSERVICE(\"https://example.com\")".into(),
        cached_value: None,
    };
    active.recalculation.status = RecalculationStatus::Stale;
    active.recalculation.qualified = false;
    assert!(validate_workbook(&active)
        .unwrap_err()
        .contains("external or active"));
}

#[test]
fn rust_matches_shared_typescript_contract_vectors() {
    let vectors: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../src/lib/artifacts/workbooks/vectors.json"
    ))
    .unwrap();
    let valid = vectors.get("valid").unwrap().clone();
    let workbook: WorkbookIr = serde_json::from_value(valid.clone()).unwrap();
    validate_workbook(&workbook).unwrap();
    for vector in vectors.get("invalid").unwrap().as_array().unwrap() {
        let mut candidate = valid.clone();
        for mutation in vector.get("mutations").unwrap().as_array().unwrap() {
            set_json_path(
                &mut candidate,
                mutation.get("path").unwrap().as_array().unwrap(),
                mutation.get("value").unwrap().clone(),
            );
        }
        let rejected = serde_json::from_value::<WorkbookIr>(candidate)
            .map_or(true, |workbook| validate_workbook(&workbook).is_err());
        assert!(
            rejected,
            "shared invalid vector {:?} was accepted",
            vector.get("case")
        );
    }
}

#[test]
fn formulas_respect_declared_sheet_bounds_without_misreading_log10() {
    let mut workbook = deterministic_fixture().unwrap();
    let formula = workbook.worksheets[0]
        .cells
        .iter_mut()
        .find(|cell| cell.address == "B5")
        .unwrap();
    formula.value = CellValue::Formula {
        expression: "LOG10(B2)".into(),
        cached_value: None,
    };
    workbook.recalculation = RecalculationState {
        status: RecalculationStatus::Stale,
        ..RecalculationState::default()
    };
    validate_workbook(&workbook).unwrap();
    let formula = workbook.worksheets[0]
        .cells
        .iter_mut()
        .find(|cell| cell.address == "B5")
        .unwrap();
    formula.value = CellValue::Formula {
        expression: "'Source Notes'!G1".into(),
        cached_value: None,
    };
    assert!(validate_workbook(&workbook)
        .unwrap_err()
        .contains("declared worksheet bounds"));
}

#[test]
fn huge_formula_ranges_fail_before_bounded_recalculation() {
    let mut workbook = deterministic_fixture().unwrap();
    let formula = workbook.worksheets[0]
        .cells
        .iter_mut()
        .find(|cell| cell.address == "B5")
        .unwrap();
    formula.value = CellValue::Formula {
        expression: "SUM(A1:XFD1048576)".to_string(),
        cached_value: None,
    };
    workbook.worksheets[0].bounds = WorksheetBounds {
        row_count: 1_048_576,
        column_count: 16_384,
    };
    workbook.recalculation = RecalculationState {
        status: RecalculationStatus::Stale,
        ..RecalculationState::default()
    };
    assert!(validate_workbook(&workbook)
        .unwrap_err()
        .contains("evaluation budget"));
    assert!(super::super::recalculate_supported_formulas(&mut workbook)
        .unwrap_err()
        .contains("evaluation budget"));
}

#[test]
fn aggregate_formula_and_chart_work_are_bounded() {
    let mut formulas = deterministic_fixture().unwrap();
    formulas.worksheets[1].bounds.row_count = 2_100;
    formulas.worksheets[1].bounds.column_count = 16_384;
    for row in 1..=2_049 {
        formulas.worksheets[1].cells.push(WorkbookCell {
            address: format!("XFD{row}"),
            value: CellValue::Formula {
                expression: "1+1".to_string(),
                cached_value: None,
            },
            format_id: None,
            comment: None,
            provenance: vec![],
        });
    }
    formulas.recalculation = RecalculationState {
        status: RecalculationStatus::Stale,
        ..RecalculationState::default()
    };
    assert!(validate_workbook(&formulas)
        .unwrap_err()
        .contains("formula count"));

    let mut chart_points = deterministic_fixture().unwrap();
    chart_points.worksheets[0].bounds.row_count = 10_001;
    let base = chart_points.worksheets[0].charts[0].clone();
    chart_points.worksheets[0].charts = (0..4)
        .map(|chart_index| {
            let mut chart = base.clone();
            chart.chart_id = format!("aggregate_points_{chart_index}");
            chart.category_range = "'Quarterly Sales'!A2:A10001".to_string();
            chart.series = (0..32)
                .map(|series_index| {
                    let mut series = base.series[0].clone();
                    series.name = format!("Series {series_index}");
                    series.value_range = "'Quarterly Sales'!B2:B10001".to_string();
                    series
                })
                .collect();
            chart
        })
        .collect();
    assert!(validate_workbook(&chart_points)
        .unwrap_err()
        .contains("chart point budget"));

    let mut chart_series = deterministic_fixture().unwrap();
    let base = chart_series.worksheets[0].charts[0].clone();
    chart_series.worksheets[0].charts = (0..129)
        .map(|chart_index| {
            let mut chart = base.clone();
            chart.chart_id = format!("aggregate_series_{chart_index}");
            chart.category_range = "'Quarterly Sales'!A2".to_string();
            chart.series = (0..32)
                .map(|series_index| {
                    let mut series = base.series[0].clone();
                    series.name = format!("Series {series_index}");
                    series.value_range = "'Quarterly Sales'!B2".to_string();
                    series
                })
                .collect();
            chart
        })
        .collect();
    assert!(validate_workbook(&chart_series)
        .unwrap_err()
        .contains("chart series budget"));
}

fn set_json_path(
    target: &mut serde_json::Value,
    path: &[serde_json::Value],
    value: serde_json::Value,
) {
    if path.len() == 1 {
        match &path[0] {
            serde_json::Value::String(key) => {
                target.as_object_mut().unwrap().insert(key.clone(), value);
            }
            serde_json::Value::Number(index) => {
                target.as_array_mut().unwrap()[index.as_u64().unwrap() as usize] = value;
            }
            _ => unreachable!(),
        };
        return;
    }
    let next = match &path[0] {
        serde_json::Value::String(key) => target.get_mut(key).unwrap(),
        serde_json::Value::Number(index) => {
            &mut target.as_array_mut().unwrap()[index.as_u64().unwrap() as usize]
        }
        _ => unreachable!(),
    };
    set_json_path(next, &path[1..], value);
}
