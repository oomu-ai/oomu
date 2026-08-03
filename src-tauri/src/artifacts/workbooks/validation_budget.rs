use super::{CellValue, ValidationRule, WorkbookIr};

pub(crate) fn estimated_text_bytes(workbook: &WorkbookIr) -> usize {
    let mut total = workbook.title.len() + workbook.locale.len();
    for format in &workbook.formats {
        total = total.saturating_add(format.format_id.len());
        total = total.saturating_add(format.font.color.as_ref().map_or(0, String::len));
        total = total.saturating_add(format.fill_color.as_ref().map_or(0, String::len));
        total = total.saturating_add(format.number_format.as_ref().map_or(0, String::len));
    }
    for sheet in &workbook.worksheets {
        total = total.saturating_add(sheet.name.len() + sheet.sheet_id.len());
        total = total.saturating_add(sheet.merged_ranges.iter().map(String::len).sum::<usize>());
        total = total.saturating_add(
            sheet
                .column_widths
                .iter()
                .map(|width| width.column.len())
                .sum::<usize>(),
        );
        for cell in &sheet.cells {
            total = total.saturating_add(
                cell.address.len() + cell.format_id.as_ref().map_or(0, String::len),
            );
            total = total.saturating_add(match &cell.value {
                CellValue::Text { value } => value.len(),
                CellValue::Date { iso } => iso.len(),
                CellValue::Formula {
                    expression,
                    cached_value,
                } => {
                    expression.len()
                        + cached_value.as_ref().map_or(0, |value| match value {
                            super::FormulaResult::Text { value } => value.len(),
                            super::FormulaResult::Error { code } => code.len(),
                            _ => 8,
                        })
                }
                _ => 8,
            });
            if let Some(comment) = &cell.comment {
                total = total.saturating_add(comment.author.len() + comment.text.len());
            }
            total = total.saturating_add(
                cell.provenance
                    .iter()
                    .map(|source| {
                        source.source_ref.len()
                            + source.evidence_ref.len()
                            + source.note.as_ref().map_or(0, String::len)
                    })
                    .sum::<usize>(),
            );
        }
        for table in &sheet.tables {
            total = total.saturating_add(
                table.table_id.len()
                    + table.name.len()
                    + table.range.len()
                    + table.style.len()
                    + table.columns.iter().map(String::len).sum::<usize>(),
            );
        }
        for validation in &sheet.validations {
            total = total.saturating_add(
                validation.validation_id.len()
                    + validation.range.len()
                    + validation.prompt.as_ref().map_or(0, String::len)
                    + validation.error.as_ref().map_or(0, String::len),
            );
            total = total.saturating_add(match &validation.rule {
                ValidationRule::List { values } => values.iter().map(String::len).sum(),
                ValidationRule::Date {
                    minimum_iso,
                    maximum_iso,
                } => minimum_iso.len() + maximum_iso.len(),
                ValidationRule::CustomFormula { formula } => formula.len(),
                _ => 16,
            });
        }
        for chart in &sheet.charts {
            total = total.saturating_add(
                chart.chart_id.len()
                    + chart.title.len()
                    + chart.category_range.len()
                    + chart
                        .series
                        .iter()
                        .map(|series| series.name.len() + series.value_range.len())
                        .sum::<usize>(),
            );
        }
    }
    for range in &workbook.named_ranges {
        total = total.saturating_add(
            range.name.len() + range.formula.len() + range.comment.as_ref().map_or(0, String::len),
        );
    }
    total.saturating_add(
        workbook
            .recalculation
            .engine
            .as_ref()
            .map_or(0, String::len)
            + workbook
                .recalculation
                .engine_version
                .as_ref()
                .map_or(0, String::len)
            + workbook
                .recalculation
                .input_digest
                .as_ref()
                .map_or(0, String::len),
    )
}
