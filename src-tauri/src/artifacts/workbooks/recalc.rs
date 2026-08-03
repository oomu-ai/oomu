use super::{
    address::{parse_cell_address, split_qualified_range},
    validate_workbook, CellValue, FormulaResult, RecalculationState, RecalculationStatus,
    WorkbookIr, WORKBOOK_RECALC_ENGINE, WORKBOOK_RECALC_ENGINE_VERSION,
};
use crate::foundation::digest::sha256_hex;
use std::collections::{HashMap, HashSet};

type NumericCells = HashMap<(String, u32, u32), f64>;
type FormulaCells = HashSet<(String, u32, u32)>;

pub fn recalculate_supported_formulas(
    workbook: &mut WorkbookIr,
) -> Result<RecalculationState, String> {
    if !workbook
        .worksheets
        .iter()
        .flat_map(|sheet| &sheet.cells)
        .any(|cell| matches!(cell.value, CellValue::Formula { .. }))
    {
        workbook.recalculation = RecalculationState::default();
        return Ok(workbook.recalculation.clone());
    }
    workbook.recalculation = RecalculationState {
        status: RecalculationStatus::Stale,
        ..RecalculationState::default()
    };
    validate_workbook(workbook)?;
    let input_digest = formula_input_digest(workbook)?;
    let mut values = collect_numeric_inputs(workbook)?;
    let formula_cells = collect_formula_cells(workbook)?;
    let mut unresolved = workbook
        .worksheets
        .iter()
        .enumerate()
        .flat_map(|(sheet_index, sheet)| {
            sheet
                .cells
                .iter()
                .enumerate()
                .filter(|(_, cell)| matches!(cell.value, CellValue::Formula { .. }))
                .map(move |(cell_index, _)| (sheet_index, cell_index))
        })
        .collect::<Vec<_>>();
    while !unresolved.is_empty() {
        let mut next = Vec::new();
        let mut progress = false;
        for (sheet_index, cell_index) in unresolved {
            let sheet = &workbook.worksheets[sheet_index];
            let CellValue::Formula { expression, .. } = &sheet.cells[cell_index].value else {
                continue;
            };
            match evaluate_formula(expression, &sheet.name, &values, &formula_cells) {
                Ok(value) => {
                    let address = parse_cell_address(&sheet.cells[cell_index].address)?;
                    values.insert(
                        (sheet.name.to_ascii_lowercase(), address.row, address.column),
                        value,
                    );
                    if let CellValue::Formula { cached_value, .. } =
                        &mut workbook.worksheets[sheet_index].cells[cell_index].value
                    {
                        *cached_value = Some(FormulaResult::Number { value });
                    }
                    progress = true;
                }
                Err(EvaluationError::Dependency) => next.push((sheet_index, cell_index)),
                Err(EvaluationError::Unsupported(reason)) => {
                    return Err(format!(
                        "Formula {} cannot be qualified by the bounded engine: {reason}",
                        workbook.worksheets[sheet_index].cells[cell_index].address
                    ));
                }
            }
        }
        if !progress {
            return Err(
                "Formula recalculation found a cycle or unresolved dependency.".to_string(),
            );
        }
        unresolved = next;
    }
    workbook.recalculation = RecalculationState {
        status: RecalculationStatus::Recalculated,
        engine: Some(WORKBOOK_RECALC_ENGINE.to_string()),
        engine_version: Some(WORKBOOK_RECALC_ENGINE_VERSION.to_string()),
        qualified: true,
        recalculated_at_ms: Some(crate::foundation::clock::unix_time_ms_i64()),
        input_digest: Some(input_digest),
    };
    validate_workbook(workbook)?;
    Ok(workbook.recalculation.clone())
}

fn formula_input_digest(workbook: &WorkbookIr) -> Result<String, String> {
    let mut canonical = workbook.clone();
    canonical.recalculation = RecalculationState {
        status: RecalculationStatus::Stale,
        ..RecalculationState::default()
    };
    for cell in canonical
        .worksheets
        .iter_mut()
        .flat_map(|sheet| &mut sheet.cells)
    {
        if let CellValue::Formula { cached_value, .. } = &mut cell.value {
            *cached_value = None;
        }
    }
    let bytes = serde_json::to_vec(&canonical).map_err(|error| error.to_string())?;
    Ok(sha256_hex(&bytes))
}

fn collect_numeric_inputs(workbook: &WorkbookIr) -> Result<NumericCells, String> {
    let mut result = HashMap::new();
    for sheet in &workbook.worksheets {
        for cell in &sheet.cells {
            let CellValue::Number { value } = cell.value else {
                continue;
            };
            let address = parse_cell_address(&cell.address)?;
            result.insert(
                (sheet.name.to_ascii_lowercase(), address.row, address.column),
                value,
            );
        }
    }
    Ok(result)
}

fn collect_formula_cells(workbook: &WorkbookIr) -> Result<FormulaCells, String> {
    let mut result = HashSet::new();
    for sheet in &workbook.worksheets {
        for cell in &sheet.cells {
            if matches!(cell.value, CellValue::Formula { .. }) {
                let address = parse_cell_address(&cell.address)?;
                result.insert((sheet.name.to_ascii_lowercase(), address.row, address.column));
            }
        }
    }
    Ok(result)
}

#[derive(Debug)]
enum EvaluationError {
    Dependency,
    Unsupported(String),
}

fn evaluate_formula(
    expression: &str,
    current_sheet: &str,
    values: &NumericCells,
    formula_cells: &FormulaCells,
) -> Result<f64, EvaluationError> {
    let formula = expression.strip_prefix('=').unwrap_or(expression).trim();
    if let Some((function, argument)) = parse_function(formula) {
        let (sheet, range) =
            split_qualified_range(argument, current_sheet).map_err(EvaluationError::Unsupported)?;
        let mut candidates = Vec::new();
        for row in range.start.row..=range.end.row {
            for column in range.start.column..=range.end.column {
                let key = (sheet.to_ascii_lowercase(), row, column);
                if formula_cells.contains(&key) && !values.contains_key(&key) {
                    return Err(EvaluationError::Dependency);
                }
                if let Some(value) = values.get(&key) {
                    candidates.push(*value);
                }
            }
        }
        if candidates.is_empty() {
            return match function.as_str() {
                "SUM" | "MIN" | "MAX" => Ok(0.0),
                "AVERAGE" => Err(EvaluationError::Unsupported(
                    "AVERAGE over an empty numeric range produces #DIV/0!".to_string(),
                )),
                _ => unreachable!(),
            };
        }
        return match function.as_str() {
            "SUM" => Ok(candidates.into_iter().sum()),
            "AVERAGE" => Ok(candidates.iter().sum::<f64>() / candidates.len() as f64),
            "MIN" => Ok(candidates.into_iter().fold(f64::INFINITY, f64::min)),
            "MAX" => Ok(candidates.into_iter().fold(f64::NEG_INFINITY, f64::max)),
            _ => Err(EvaluationError::Unsupported(format!(
                "function {function} is outside the qualified subset"
            ))),
        };
    }
    if let Some((left, operator, right)) = split_binary(formula) {
        let left = resolve_operand(left, current_sheet, values, formula_cells)?;
        let right = resolve_operand(right, current_sheet, values, formula_cells)?;
        return match operator {
            '+' => Ok(left + right),
            '-' => Ok(left - right),
            '*' => Ok(left * right),
            '/' if right != 0.0 => Ok(left / right),
            '/' => Err(EvaluationError::Unsupported("division by zero".to_string())),
            _ => Err(EvaluationError::Unsupported(
                "operator is outside the qualified subset".to_string(),
            )),
        };
    }
    resolve_operand(formula, current_sheet, values, formula_cells)
}

fn parse_function(value: &str) -> Option<(String, &str)> {
    let open = value.find('(')?;
    if !value.ends_with(')') || value[open + 1..value.len() - 1].contains(',') {
        return None;
    }
    let function = value[..open].trim().to_ascii_uppercase();
    if !["SUM", "AVERAGE", "MIN", "MAX"].contains(&function.as_str()) {
        return None;
    }
    Some((function, value[open + 1..value.len() - 1].trim()))
}

fn split_binary(value: &str) -> Option<(&str, char, &str)> {
    let mut quoted = false;
    let mut depth = 0_u32;
    for (index, character) in value.char_indices() {
        match character {
            '\'' => quoted = !quoted,
            '(' if !quoted => depth += 1,
            ')' if !quoted => depth = depth.saturating_sub(1),
            '+' | '-' | '*' | '/' if !quoted && depth == 0 && index > 0 => {
                return Some((&value[..index], character, &value[index + 1..]));
            }
            _ => {}
        }
    }
    None
}

fn resolve_operand(
    raw: &str,
    current_sheet: &str,
    values: &NumericCells,
    formula_cells: &FormulaCells,
) -> Result<f64, EvaluationError> {
    let value = raw.trim();
    if let Ok(number) = value.parse::<f64>() {
        return if number.is_finite() {
            Ok(number)
        } else {
            Err(EvaluationError::Unsupported(
                "non-finite literal".to_string(),
            ))
        };
    }
    let (sheet, range) =
        split_qualified_range(value, current_sheet).map_err(EvaluationError::Unsupported)?;
    if range.start != range.end {
        return Err(EvaluationError::Unsupported(
            "range operands require an aggregate function".to_string(),
        ));
    }
    let key = (
        sheet.to_ascii_lowercase(),
        range.start.row,
        range.start.column,
    );
    if formula_cells.contains(&key) && !values.contains_key(&key) {
        return Err(EvaluationError::Dependency);
    }
    values
        .get(&key)
        .copied()
        .ok_or_else(|| EvaluationError::Unsupported("operand is blank or non-numeric".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifacts::workbooks::{deterministic_fixture, RecalculationStatus};

    #[test]
    fn qualified_subset_recalculates_fixture() {
        let workbook = deterministic_fixture().unwrap();
        assert_eq!(
            workbook.recalculation.status,
            RecalculationStatus::Recalculated
        );
        let formula = workbook.worksheets[0]
            .cells
            .iter()
            .find(|cell| matches!(cell.value, CellValue::Formula { .. }))
            .unwrap();
        assert_eq!(
            formula.value,
            CellValue::Formula {
                expression: "SUM(B2:B4)".into(),
                cached_value: Some(FormulaResult::Number { value: 3900.0 })
            }
        );
    }

    #[test]
    fn aggregate_waits_for_out_of_order_formula_dependencies() {
        let mut workbook = deterministic_fixture().unwrap();
        let sheet = &mut workbook.worksheets[0];
        let b4 = sheet
            .cells
            .iter()
            .position(|cell| cell.address == "B4")
            .unwrap();
        sheet.cells[b4].value = CellValue::Formula {
            expression: "B3*2".into(),
            cached_value: None,
        };
        let b5 = sheet
            .cells
            .iter()
            .position(|cell| cell.address == "B5")
            .unwrap();
        sheet.cells[b5].value = CellValue::Formula {
            expression: "SUM(B2:B4)".into(),
            cached_value: None,
        };
        sheet.cells.swap(b4, b5);
        workbook.recalculation = RecalculationState {
            status: RecalculationStatus::Stale,
            ..RecalculationState::default()
        };
        recalculate_supported_formulas(&mut workbook).unwrap();
        let total = workbook.worksheets[0]
            .cells
            .iter()
            .find(|cell| cell.address == "B5")
            .unwrap();
        assert_eq!(
            total.value,
            CellValue::Formula {
                expression: "SUM(B2:B4)".into(),
                cached_value: Some(FormulaResult::Number { value: 5_550.0 })
            }
        );
    }

    #[test]
    fn cycles_fail_and_empty_aggregates_follow_excel_semantics() {
        let mut cycle = deterministic_fixture().unwrap();
        for (address, expression) in [("B3", "B4+1"), ("B4", "B3+1")] {
            let cell = cycle.worksheets[0]
                .cells
                .iter_mut()
                .find(|cell| cell.address == address)
                .unwrap();
            cell.value = CellValue::Formula {
                expression: expression.into(),
                cached_value: None,
            };
        }
        cycle.recalculation = RecalculationState {
            status: RecalculationStatus::Stale,
            ..RecalculationState::default()
        };
        assert!(recalculate_supported_formulas(&mut cycle)
            .unwrap_err()
            .contains("cycle"));

        let mut empty = deterministic_fixture().unwrap();
        empty.worksheets[0].cells.push(super::super::WorkbookCell {
            address: "B6".into(),
            value: CellValue::Formula {
                expression: "SUM(E2:E4)".into(),
                cached_value: None,
            },
            format_id: None,
            comment: None,
            provenance: vec![],
        });
        empty.recalculation = RecalculationState {
            status: RecalculationStatus::Stale,
            ..RecalculationState::default()
        };
        recalculate_supported_formulas(&mut empty).unwrap();
        assert_eq!(
            empty.worksheets[0]
                .cells
                .iter()
                .find(|cell| cell.address == "B6")
                .unwrap()
                .value,
            CellValue::Formula {
                expression: "SUM(E2:E4)".into(),
                cached_value: Some(FormulaResult::Number { value: 0.0 })
            }
        );
    }
}
