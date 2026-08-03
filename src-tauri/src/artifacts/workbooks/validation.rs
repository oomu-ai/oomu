use super::{
    address::{
        extract_formula_references, formula_is_external_or_active, parse_cell_address,
        parse_local_range, split_qualified_range, CellRange,
    },
    validation_primitives::{
        bounded_identifier, bounded_text, parse_date_value, reject_controls, validate_date,
        validate_recalculation,
    },
    CellValue, ValidationRule, WorkbookIr, WORKBOOK_IR_SCHEMA_VERSION,
};
use regex::Regex;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub struct ValidatedWorkbook(pub WorkbookIr);

const MAX_REFERENCED_CELLS_PER_FORMULA: u64 = 100_000;
const MAX_REFERENCED_CELLS_PER_WORKBOOK: u64 = 2_000_000;
const MAX_FORMULAS_PER_WORKBOOK: usize = 2_048;
const MAX_CHART_SERIES_PER_WORKBOOK: usize = 4_096;
const MAX_CHART_POINTS_PER_WORKBOOK: u64 = 1_000_000;

pub fn validate_workbook(workbook: &WorkbookIr) -> Result<ValidatedWorkbook, String> {
    if workbook.schema_version != WORKBOOK_IR_SCHEMA_VERSION {
        return Err("Unsupported workbook IR schema version.".to_string());
    }
    bounded_text(&workbook.title, 1, 240, "Workbook title")?;
    let locale = Regex::new(r"^[A-Za-z]{2,3}(?:-[A-Za-z0-9]{2,8})*$").map_err(|e| e.to_string())?;
    if !locale.is_match(&workbook.locale) {
        return Err("Workbook locale must be a valid BCP-47 style tag.".to_string());
    }
    if workbook.revision == 0 || workbook.worksheets.is_empty() || workbook.worksheets.len() > 1_024
    {
        return Err("Workbook revision and worksheet count are invalid.".to_string());
    }
    if workbook.formats.len() > 10_000 || workbook.named_ranges.len() > 10_000 {
        return Err("Workbook format or named-range count exceeds the safe limit.".to_string());
    }
    let total_cells = workbook
        .worksheets
        .iter()
        .map(|sheet| sheet.cells.len())
        .sum::<usize>();
    if total_cells > 2_000_000
        || super::validation_budget::estimated_text_bytes(workbook) > 64 * 1024 * 1024
    {
        return Err("Workbook exceeds the bounded cell or text budget.".to_string());
    }
    validate_formats(workbook)?;
    let sheet_names = workbook
        .worksheets
        .iter()
        .map(|sheet| sheet.name.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    if sheet_names.len() != workbook.worksheets.len() {
        return Err("Worksheet names must be unique ignoring case.".to_string());
    }
    let mut sheet_ids = HashSet::new();
    let sheet_bounds = workbook
        .worksheets
        .iter()
        .map(|sheet| (sheet.name.to_ascii_lowercase(), sheet.bounds))
        .collect::<HashMap<_, _>>();
    let mut table_names = HashSet::new();
    for table in workbook.worksheets.iter().flat_map(|sheet| &sheet.tables) {
        if !table_names.insert(table.name.to_ascii_lowercase()) {
            return Err(format!(
                "Table name {} must be unique workbook-wide.",
                table.name
            ));
        }
    }
    for sheet in &workbook.worksheets {
        if !sheet_ids.insert(sheet.sheet_id.to_ascii_lowercase()) {
            return Err("Worksheet identifiers must be unique.".to_string());
        }
        validate_sheet(workbook, sheet, &sheet_names, &sheet_bounds)?;
    }
    validate_aggregate_chart_budget(workbook)?;
    validate_named_ranges(workbook, &sheet_names, &sheet_bounds)?;
    validate_formula_reference_budget(workbook)?;
    validate_recalculation(workbook)?;
    Ok(ValidatedWorkbook(workbook.clone()))
}

fn validate_formula_reference_budget(workbook: &WorkbookIr) -> Result<(), String> {
    let mut aggregate = 0_u64;
    let mut formula_count = 0_usize;
    let formulas = workbook
        .worksheets
        .iter()
        .flat_map(|sheet| {
            let cell_formulas = sheet.cells.iter().filter_map(|cell| match &cell.value {
                CellValue::Formula { expression, .. } => {
                    Some((sheet.name.as_str(), expression.as_str()))
                }
                _ => None,
            });
            let validation_formulas =
                sheet
                    .validations
                    .iter()
                    .filter_map(|validation| match &validation.rule {
                        ValidationRule::CustomFormula { formula } => {
                            Some((sheet.name.as_str(), formula.as_str()))
                        }
                        _ => None,
                    });
            cell_formulas.chain(validation_formulas)
        })
        .chain(
            workbook
                .named_ranges
                .iter()
                .map(|named| (workbook.worksheets[0].name.as_str(), named.formula.as_str())),
        );
    for (sheet, formula) in formulas {
        formula_count += 1;
        if formula_count > MAX_FORMULAS_PER_WORKBOOK {
            return Err("Workbook exceeds the bounded formula count.".to_string());
        }
        let mut per_formula = 0_u64;
        for (_, range) in extract_formula_references(formula, sheet)? {
            per_formula = per_formula
                .checked_add(range.cell_count())
                .ok_or_else(|| "Formula reference budget overflowed.".to_string())?;
        }
        if per_formula > MAX_REFERENCED_CELLS_PER_FORMULA {
            return Err("Formula exceeds the bounded reference evaluation budget.".to_string());
        }
        aggregate = aggregate
            .checked_add(per_formula)
            .ok_or_else(|| "Workbook formula reference budget overflowed.".to_string())?;
        if aggregate > MAX_REFERENCED_CELLS_PER_WORKBOOK {
            return Err("Workbook exceeds the aggregate formula evaluation budget.".to_string());
        }
    }
    Ok(())
}

fn validate_aggregate_chart_budget(workbook: &WorkbookIr) -> Result<(), String> {
    let mut series_count = 0_usize;
    let mut point_work = 0_u64;
    for chart in workbook.worksheets.iter().flat_map(|sheet| &sheet.charts) {
        series_count = series_count
            .checked_add(chart.series.len())
            .ok_or_else(|| "Workbook chart series budget overflowed.".to_string())?;
        if series_count > MAX_CHART_SERIES_PER_WORKBOOK {
            return Err("Workbook exceeds the aggregate chart series budget.".to_string());
        }
        let (_, categories) = split_qualified_range(&chart.category_range, "")?;
        point_work = point_work
            .checked_add(
                categories
                    .cell_count()
                    .checked_mul(chart.series.len() as u64)
                    .ok_or_else(|| "Workbook chart point budget overflowed.".to_string())?,
            )
            .ok_or_else(|| "Workbook chart point budget overflowed.".to_string())?;
        if point_work > MAX_CHART_POINTS_PER_WORKBOOK {
            return Err("Workbook exceeds the aggregate chart point budget.".to_string());
        }
    }
    Ok(())
}

fn validate_formats(workbook: &WorkbookIr) -> Result<(), String> {
    let mut ids = HashSet::new();
    let color = Regex::new(r"^[0-9A-Fa-f]{6}$").map_err(|e| e.to_string())?;
    for format in &workbook.formats {
        bounded_identifier(&format.format_id, "Format identifier")?;
        if !ids.insert(format.format_id.as_str()) {
            return Err(format!("Duplicate format identifier {}.", format.format_id));
        }
        for candidate in [format.fill_color.as_ref(), format.font.color.as_ref()]
            .into_iter()
            .flatten()
        {
            if !color.is_match(candidate) {
                return Err(format!(
                    "Format {} contains an invalid RGB color.",
                    format.format_id
                ));
            }
        }
        if let Some(size) = format.font.size_pt {
            if !size.is_finite() || !(6.0..=72.0).contains(&size) {
                return Err(format!(
                    "Format {} has an invalid font size.",
                    format.format_id
                ));
            }
        }
        if let Some(number_format) = &format.number_format {
            bounded_text(number_format, 1, 160, "Number format")?;
            reject_controls(number_format, "Number format")?;
        }
    }
    Ok(())
}

fn validate_sheet(
    workbook: &WorkbookIr,
    sheet: &super::Worksheet,
    sheet_names: &HashSet<String>,
    sheet_bounds: &HashMap<String, super::WorksheetBounds>,
) -> Result<(), String> {
    bounded_identifier(&sheet.sheet_id, "Worksheet identifier")?;
    if sheet.name.chars().count() > 31
        || sheet.name.trim().is_empty()
        || sheet
            .name
            .chars()
            .any(|value| matches!(value, '[' | ']' | ':' | '*' | '?' | '/' | '\\'))
        || sheet.name.starts_with('\'')
        || sheet.name.ends_with('\'')
    {
        return Err(format!("Worksheet name {} is not Excel-safe.", sheet.name));
    }
    if sheet.critical && !matches!(sheet.visibility, super::SheetVisibility::Visible) {
        return Err(format!(
            "Critical worksheet {} must remain visible.",
            sheet.name
        ));
    }
    if sheet.bounds.row_count == 0
        || sheet.bounds.row_count > 1_048_576
        || sheet.bounds.column_count == 0
        || sheet.bounds.column_count > 16_384
    {
        return Err(format!(
            "Worksheet {} has invalid declared bounds.",
            sheet.name
        ));
    }
    if sheet.cells.len() > 1_000_000 {
        return Err(format!("Worksheet {} exceeds the cell limit.", sheet.name));
    }
    if sheet.merged_ranges.len() > 10_000
        || sheet.column_widths.len() > 16_384
        || sheet.tables.len() > 1_024
        || sheet.validations.len() > 10_000
        || sheet.charts.len() > 256
    {
        return Err(format!(
            "Worksheet {} exceeds a structural collection limit.",
            sheet.name
        ));
    }
    let format_ids = workbook
        .formats
        .iter()
        .map(|value| value.format_id.as_str())
        .collect::<HashSet<_>>();
    let mut addresses = HashSet::new();
    let mut cell_index = HashMap::with_capacity(sheet.cells.len());
    for cell in &sheet.cells {
        let address = parse_cell_address(&cell.address)?;
        if address.row > sheet.bounds.row_count || address.column > sheet.bounds.column_count {
            return Err(format!(
                "Cell {} exceeds the declared bounds for {}.",
                cell.address, sheet.name
            ));
        }
        if !addresses.insert((address.row, address.column)) {
            return Err(format!(
                "Worksheet {} contains duplicate cell {}.",
                sheet.name, cell.address
            ));
        }
        cell_index.insert((address.row, address.column), cell);
        if let Some(format_id) = &cell.format_id {
            if !format_ids.contains(format_id.as_str()) {
                return Err(format!(
                    "Cell {} references missing format {}.",
                    cell.address, format_id
                ));
            }
        }
        validate_cell_value(&cell.value, &sheet.name, sheet_names, sheet_bounds)?;
        if let Some(comment) = &cell.comment {
            bounded_text(&comment.author, 1, 160, "Comment author")?;
            bounded_text(&comment.text, 1, 32_000, "Comment text")?;
        }
        for source in &cell.provenance {
            bounded_identifier(&source.source_ref, "Source reference")?;
            bounded_identifier(&source.evidence_ref, "Evidence reference")?;
            if let Some(note) = &source.note {
                bounded_text(note, 1, 1_000, "Provenance note")?;
            }
        }
        if cell.provenance.len() > 64 {
            return Err(format!(
                "Cell {} has too many provenance references.",
                cell.address
            ));
        }
    }
    validate_ranges(sheet)?;
    validate_tables(sheet, &cell_index)?;
    validate_validations(sheet, sheet_names, sheet_bounds)?;
    validate_charts(sheet, sheet_names, sheet_bounds)?;
    Ok(())
}

fn validate_cell_value(
    value: &CellValue,
    sheet: &str,
    sheet_names: &HashSet<String>,
    sheet_bounds: &HashMap<String, super::WorksheetBounds>,
) -> Result<(), String> {
    match value {
        CellValue::Blank => Ok(()),
        CellValue::Text { value } => {
            if value.chars().count() > 32_767 {
                return Err("Cell text exceeds Excel's limit.".to_string());
            }
            reject_controls(value, "Cell text")
        }
        CellValue::Number { value } if value.is_finite() => Ok(()),
        CellValue::Number { .. } => Err("Cell numbers must be finite.".to_string()),
        CellValue::Boolean { .. } => Ok(()),
        CellValue::Date { iso } => validate_date(iso),
        CellValue::Formula {
            expression,
            cached_value,
        } => {
            validate_formula(expression, sheet, sheet_names, sheet_bounds)?;
            if let Some(super::FormulaResult::Number { value }) = cached_value {
                if !value.is_finite() {
                    return Err("Formula cached numbers must be finite.".to_string());
                }
            }
            if let Some(super::FormulaResult::Error { code }) = cached_value {
                const VALID: [&str; 8] = [
                    "#NULL!",
                    "#DIV/0!",
                    "#VALUE!",
                    "#REF!",
                    "#NAME?",
                    "#NUM!",
                    "#N/A",
                    "#GETTING_DATA",
                ];
                if !VALID.contains(&code.as_str()) {
                    return Err("Formula cached error code is invalid.".to_string());
                }
            }
            Ok(())
        }
    }
}

fn validate_formula(
    expression: &str,
    sheet: &str,
    sheet_names: &HashSet<String>,
    sheet_bounds: &HashMap<String, super::WorksheetBounds>,
) -> Result<(), String> {
    let formula = expression.strip_prefix('=').unwrap_or(expression).trim();
    bounded_text(formula, 1, 8_192, "Formula")?;
    reject_controls(formula, "Formula")?;
    if formula_is_external_or_active(formula) {
        return Err("Formula contains an external or active-content reference.".to_string());
    }
    for (referenced_sheet, range) in extract_formula_references(formula, sheet)? {
        if !sheet_names.contains(&referenced_sheet.to_ascii_lowercase()) {
            return Err(format!(
                "Formula references missing worksheet {referenced_sheet}."
            ));
        }
        let bounds = sheet_bounds
            .get(&referenced_sheet.to_ascii_lowercase())
            .ok_or_else(|| format!("Formula references missing worksheet {referenced_sheet}."))?;
        require_within_bounds(range, *bounds, "Formula reference")?;
    }
    Ok(())
}

fn validate_ranges(sheet: &super::Worksheet) -> Result<(), String> {
    let mut merged = Vec::new();
    for range in &sheet.merged_ranges {
        let parsed = parse_local_range(range)?;
        if parsed.cell_count() < 2 {
            return Err("Merged ranges must contain multiple cells.".to_string());
        }
        require_within_bounds(parsed, sheet.bounds, "Merged range")?;
        if merged
            .iter()
            .any(|existing| ranges_overlap(*existing, parsed))
        {
            return Err("Merged ranges may not overlap each other.".to_string());
        }
        if sheet
            .tables
            .iter()
            .map(|table| parse_local_range(&table.range))
            .collect::<Result<Vec<_>, _>>()?
            .iter()
            .any(|table| ranges_overlap(*table, parsed))
        {
            return Err("Merged ranges may not overlap tables.".to_string());
        }
        merged.push(parsed);
    }
    let mut configured_columns = HashSet::new();
    for width in &sheet.column_widths {
        let address = parse_cell_address(&format!("{}1", width.column))?;
        if !configured_columns.insert(address.column) {
            return Err(format!(
                "Column {} has duplicate width declarations.",
                width.column
            ));
        }
        if address.row != 1
            || address.column > sheet.bounds.column_count
            || !width.width.is_finite()
            || !(1.0..=255.0).contains(&width.width)
        {
            return Err(format!("Column width {} is invalid.", width.column));
        }
    }
    Ok(())
}

fn validate_tables(
    sheet: &super::Worksheet,
    cell_index: &HashMap<(u32, u32), &super::WorkbookCell>,
) -> Result<(), String> {
    let mut ids = HashSet::new();
    let mut names = HashSet::new();
    let name_pattern = Regex::new(r"^[A-Za-z_][A-Za-z0-9_.]{0,254}$").map_err(|e| e.to_string())?;
    let mut ranges = Vec::new();
    for table in &sheet.tables {
        bounded_identifier(&table.table_id, "Table identifier")?;
        if !ids.insert(table.table_id.to_ascii_lowercase())
            || !names.insert(table.name.to_ascii_lowercase())
        {
            return Err("Table identifiers and names must be unique per sheet.".to_string());
        }
        if !name_pattern.is_match(&table.name)
            || invalid_excel_name(&table.name)
            || table.columns.is_empty()
        {
            return Err(format!(
                "Table {} has an invalid name or no columns.",
                table.name
            ));
        }
        let range = parse_local_range(&table.range)?;
        require_within_bounds(range, sheet.bounds, "Table range")?;
        if ranges
            .iter()
            .any(|existing| ranges_overlap(*existing, range))
        {
            return Err("Tables may not overlap.".to_string());
        }
        if range.width() as usize != table.columns.len() || range.end.row <= range.start.row {
            return Err(format!(
                "Table {} range does not match its columns.",
                table.name
            ));
        }
        for (offset, expected) in table.columns.iter().enumerate() {
            let location = super::address::CellAddress {
                row: range.start.row,
                column: range.start.column + offset as u32,
            };
            let address = super::address::a1(location);
            let header = cell_index
                .get(&(location.row, location.column))
                .ok_or_else(|| format!("Table {} is missing header cell {address}.", table.name))?;
            match &header.value {
                CellValue::Text { value } if value == expected => {}
                _ => {
                    return Err(format!(
                        "Table {} header cell {address} must exactly match column {expected}.",
                        table.name
                    ))
                }
            }
        }
        let mut column_names = HashSet::new();
        for column in &table.columns {
            bounded_text(column, 1, 255, "Table column")?;
            if !column_names.insert(column.to_ascii_lowercase()) {
                return Err(format!(
                    "Table {} contains duplicate column names.",
                    table.name
                ));
            }
        }
        bounded_text(&table.style, 1, 128, "Table style")?;
        ranges.push(range);
    }
    Ok(())
}

fn validate_validations(
    sheet: &super::Worksheet,
    sheet_names: &HashSet<String>,
    sheet_bounds: &HashMap<String, super::WorksheetBounds>,
) -> Result<(), String> {
    let mut ids = HashSet::new();
    for validation in &sheet.validations {
        bounded_identifier(&validation.validation_id, "Validation identifier")?;
        if !ids.insert(validation.validation_id.to_ascii_lowercase()) {
            return Err("Validation identifiers must be unique.".to_string());
        }
        require_within_bounds(
            parse_local_range(&validation.range)?,
            sheet.bounds,
            "Validation range",
        )?;
        match &validation.rule {
            ValidationRule::List { values } => {
                if values.is_empty() || values.len() > 100 {
                    return Err("Validation lists require 1 to 100 values.".to_string());
                }
                for value in values {
                    bounded_text(value, 1, 255, "Validation list value")?;
                }
                let serialized_length = 2
                    + values
                        .iter()
                        .map(|value| value.replace('"', "\"\"").chars().count())
                        .sum::<usize>()
                    + values.len().saturating_sub(1);
                if serialized_length > 255 {
                    return Err(
                        "Inline validation list exceeds Excel's 255-character formula limit."
                            .to_string(),
                    );
                }
            }
            ValidationRule::WholeNumber { minimum, maximum } if minimum <= maximum => {}
            ValidationRule::Decimal { minimum, maximum }
                if minimum.is_finite() && maximum.is_finite() && minimum <= maximum => {}
            ValidationRule::Date {
                minimum_iso,
                maximum_iso,
            } => {
                let minimum = parse_date_value(minimum_iso)?;
                let maximum = parse_date_value(maximum_iso)?;
                if minimum > maximum {
                    return Err("Validation date minimum exceeds maximum.".to_string());
                }
            }
            ValidationRule::CustomFormula { formula } => {
                validate_formula(formula, &sheet.name, sheet_names, sheet_bounds)?
            }
            _ => return Err("Validation bounds are invalid.".to_string()),
        }
        if let Some(prompt) = &validation.prompt {
            bounded_text(prompt, 1, 255, "Validation prompt")?;
        }
        if let Some(error) = &validation.error {
            bounded_text(error, 1, 255, "Validation error")?;
        }
    }
    Ok(())
}

fn validate_charts(
    sheet: &super::Worksheet,
    sheet_names: &HashSet<String>,
    sheet_bounds: &HashMap<String, super::WorksheetBounds>,
) -> Result<(), String> {
    let mut ids = HashSet::new();
    for chart in &sheet.charts {
        bounded_identifier(&chart.chart_id, "Chart identifier")?;
        if !ids.insert(chart.chart_id.to_ascii_lowercase())
            || chart.series.is_empty()
            || chart.series.len() > 32
        {
            return Err(
                "Chart identifiers must be unique and charts need 1 to 32 series.".to_string(),
            );
        }
        bounded_text(&chart.title, 1, 240, "Chart title")?;
        let categories = validate_qualified_range(
            &chart.category_range,
            &sheet.name,
            sheet_names,
            sheet_bounds,
        )?;
        if categories.cell_count() > 10_000 {
            return Err(format!(
                "Chart {} exceeds the 10,000-point safety limit.",
                chart.chart_id
            ));
        }
        for series in &chart.series {
            bounded_text(&series.name, 1, 240, "Chart series name")?;
            let values = validate_qualified_range(
                &series.value_range,
                &sheet.name,
                sheet_names,
                sheet_bounds,
            )?;
            if values.cell_count() != categories.cell_count() {
                return Err(format!(
                    "Chart {} category and series ranges must contain equal point counts.",
                    chart.chart_id
                ));
            }
        }
        let anchor = &chart.anchor;
        if anchor.to_column <= anchor.from_column
            || anchor.to_row <= anchor.from_row
            || anchor.to_column > sheet.bounds.column_count
            || anchor.to_row > sheet.bounds.row_count
        {
            return Err(format!("Chart {} has an invalid anchor.", chart.chart_id));
        }
    }
    Ok(())
}

fn validate_named_ranges(
    workbook: &WorkbookIr,
    sheet_names: &HashSet<String>,
    sheet_bounds: &HashMap<String, super::WorksheetBounds>,
) -> Result<(), String> {
    let pattern = Regex::new(r"^[A-Za-z_\\][A-Za-z0-9_.\\]{0,254}$").map_err(|e| e.to_string())?;
    let mut names = HashSet::new();
    for named in &workbook.named_ranges {
        if !pattern.is_match(&named.name)
            || invalid_excel_name(&named.name)
            || !names.insert(named.name.to_ascii_lowercase())
        {
            return Err(format!(
                "Named range {} is invalid or duplicated.",
                named.name
            ));
        }
        validate_formula(
            &named.formula,
            &workbook.worksheets[0].name,
            sheet_names,
            sheet_bounds,
        )?;
        if let Some(comment) = &named.comment {
            bounded_text(comment, 1, 255, "Named range comment")?;
        }
    }
    Ok(())
}

fn invalid_excel_name(value: &str) -> bool {
    let uppercase = value.to_ascii_uppercase();
    if matches!(uppercase.as_str(), "R" | "C") {
        return true;
    }
    if parse_cell_address(&uppercase).is_ok() {
        return true;
    }
    let r1c1 =
        Regex::new(r"^R(?:\[?-?[0-9]+\]?)?C(?:\[?-?[0-9]+\]?)?$").expect("static R1C1 regex");
    r1c1.is_match(&uppercase)
}

fn validate_qualified_range(
    raw: &str,
    default_sheet: &str,
    sheet_names: &HashSet<String>,
    sheet_bounds: &HashMap<String, super::WorksheetBounds>,
) -> Result<CellRange, String> {
    let (sheet, range) = split_qualified_range(raw, default_sheet)?;
    if !sheet_names.contains(&sheet.to_ascii_lowercase()) {
        return Err(format!("Range references missing worksheet {sheet}."));
    }
    let bounds = *sheet_bounds
        .get(&sheet.to_ascii_lowercase())
        .ok_or_else(|| format!("Range references missing worksheet {sheet}."))?;
    require_within_bounds(range, bounds, "Qualified range")?;
    Ok(range)
}

fn require_within_bounds(
    range: CellRange,
    bounds: super::WorksheetBounds,
    label: &str,
) -> Result<(), String> {
    if range.end.row > bounds.row_count || range.end.column > bounds.column_count {
        Err(format!("{label} exceeds declared worksheet bounds."))
    } else {
        Ok(())
    }
}

fn ranges_overlap(left: CellRange, right: CellRange) -> bool {
    left.start.row <= right.end.row
        && right.start.row <= left.end.row
        && left.start.column <= right.end.column
        && right.start.column <= left.end.column
}

#[cfg(test)]
#[path = "validation_tests.rs"]
mod tests;
