use super::{CellValue, RecalculationStatus, WorkbookIr};
use chrono::{DateTime, NaiveDate};

pub(super) fn validate_recalculation(workbook: &WorkbookIr) -> Result<(), String> {
    let has_formulas = workbook
        .worksheets
        .iter()
        .flat_map(|sheet| &sheet.cells)
        .any(|cell| matches!(cell.value, CellValue::Formula { .. }));
    match workbook.recalculation.status {
        RecalculationStatus::NotRequired if has_formulas => {
            Err("Formula workbooks cannot claim recalculation is not required.".to_string())
        }
        RecalculationStatus::Recalculated => {
            if !has_formulas
                || !workbook.recalculation.qualified
                || workbook
                    .recalculation
                    .engine
                    .as_deref()
                    .unwrap_or("")
                    .is_empty()
                || workbook
                    .recalculation
                    .engine_version
                    .as_deref()
                    .unwrap_or("")
                    .is_empty()
                || workbook.recalculation.recalculated_at_ms.is_none()
                || workbook
                    .recalculation
                    .input_digest
                    .as_deref()
                    .map_or(true, |value| value.len() != 64)
            {
                return Err(
                    "Recalculated workbooks require a complete qualified engine receipt."
                        .to_string(),
                );
            }
            let missing = workbook
                .worksheets
                .iter()
                .flat_map(|sheet| &sheet.cells)
                .any(|cell| {
                    matches!(
                        cell.value,
                        CellValue::Formula {
                            cached_value: None,
                            ..
                        }
                    )
                });
            if missing {
                return Err(
                    "Qualified recalculation requires a cached result for every formula."
                        .to_string(),
                );
            }
            Ok(())
        }
        RecalculationStatus::Stale if !has_formulas => {
            Err("Formula-free workbooks cannot be marked stale.".to_string())
        }
        _ => Ok(()),
    }
}

pub(super) fn validate_date(value: &str) -> Result<(), String> {
    parse_date_value(value).map(|_| ())
}

pub(super) fn parse_date_value(value: &str) -> Result<i64, String> {
    if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        return Ok(date
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis());
    }
    DateTime::parse_from_rfc3339(value)
        .map(|date| date.timestamp_millis())
        .map_err(|_| format!("Date {value} is not ISO-8601."))
}

pub(super) fn bounded_identifier(value: &str, field: &str) -> Result<(), String> {
    bounded_text(value, 1, 256, field)?;
    reject_controls(value, field)
}

pub(super) fn bounded_text(value: &str, min: usize, max: usize, field: &str) -> Result<(), String> {
    let length = value.chars().count();
    if length < min || length > max {
        Err(format!("{field} must contain {min} to {max} characters."))
    } else {
        Ok(())
    }
}

pub(super) fn reject_controls(value: &str, field: &str) -> Result<(), String> {
    if value
        .chars()
        .any(|character| character < ' ' && !matches!(character, '\t' | '\n' | '\r'))
    {
        Err(format!("{field} contains unsupported control characters."))
    } else {
        Ok(())
    }
}
