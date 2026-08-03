use regex::Regex;

pub(crate) const MAX_ROWS: u32 = 1_048_576;
pub(crate) const MAX_COLUMNS: u32 = 16_384;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CellAddress {
    pub row: u32,
    pub column: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CellRange {
    pub start: CellAddress,
    pub end: CellAddress,
}

impl CellRange {
    pub fn contains(&self, address: CellAddress) -> bool {
        address.row >= self.start.row
            && address.row <= self.end.row
            && address.column >= self.start.column
            && address.column <= self.end.column
    }

    pub fn width(&self) -> u32 {
        self.end.column - self.start.column + 1
    }

    pub fn cell_count(&self) -> u64 {
        u64::from(self.width()) * u64::from(self.end.row - self.start.row + 1)
    }
}

pub(crate) fn parse_cell_address(raw: &str) -> Result<CellAddress, String> {
    let normalized = raw.trim().replace('$', "");
    let split = normalized
        .find(|character: char| character.is_ascii_digit())
        .ok_or_else(|| format!("Cell address {raw} has no row."))?;
    let (column, row) = normalized.split_at(split);
    if column.is_empty()
        || column.len() > 3
        || !column.chars().all(|value| value.is_ascii_alphabetic())
        || row.is_empty()
        || !row.chars().all(|value| value.is_ascii_digit())
    {
        return Err(format!("Cell address {raw} is not valid A1 notation."));
    }
    let mut column_index = 0_u32;
    for value in column.bytes() {
        column_index = column_index
            .checked_mul(26)
            .and_then(|current| {
                current.checked_add(u32::from(value.to_ascii_uppercase() - b'A' + 1))
            })
            .ok_or_else(|| format!("Cell address {raw} exceeds Excel bounds."))?;
    }
    let row_index = row
        .parse::<u32>()
        .map_err(|_| format!("Cell address {raw} has an invalid row."))?;
    if row_index == 0 || row_index > MAX_ROWS || column_index == 0 || column_index > MAX_COLUMNS {
        return Err(format!("Cell address {raw} exceeds Excel bounds."));
    }
    Ok(CellAddress {
        row: row_index,
        column: column_index,
    })
}

pub(crate) fn parse_local_range(raw: &str) -> Result<CellRange, String> {
    if raw.contains('!') {
        return Err(format!("Range {raw} must not contain a sheet qualifier."));
    }
    let mut parts = raw.split(':');
    let start = parse_cell_address(parts.next().unwrap_or_default())?;
    let end = parse_cell_address(parts.next().unwrap_or(raw))?;
    if parts.next().is_some() || end.row < start.row || end.column < start.column {
        return Err(format!("Range {raw} is invalid or reversed."));
    }
    Ok(CellRange { start, end })
}

pub(crate) fn split_qualified_range<'a>(
    raw: &'a str,
    default_sheet: &'a str,
) -> Result<(String, CellRange), String> {
    let trimmed = raw.trim();
    let (sheet, range) = if let Some(index) = find_sheet_separator(trimmed) {
        (&trimmed[..index], &trimmed[index + 1..])
    } else {
        (default_sheet, trimmed)
    };
    let sheet = unquote_sheet_name(sheet)?;
    Ok((sheet, parse_local_range(range)?))
}

fn find_sheet_separator(value: &str) -> Option<usize> {
    let mut quoted = false;
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\'' {
            if quoted && bytes.get(index + 1) == Some(&b'\'') {
                index += 2;
                continue;
            }
            quoted = !quoted;
        } else if bytes[index] == b'!' && !quoted {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn unquote_sheet_name(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.starts_with('\'') || trimmed.ends_with('\'') {
        if trimmed.len() < 2 || !trimmed.starts_with('\'') || !trimmed.ends_with('\'') {
            return Err(format!("Sheet qualifier {raw} is malformed."));
        }
        Ok(trimmed[1..trimmed.len() - 1].replace("''", "'"))
    } else {
        Ok(trimmed.to_string())
    }
}

pub(crate) fn column_name(mut column: u32) -> String {
    let mut result = Vec::new();
    while column > 0 {
        let remainder = ((column - 1) % 26) as u8;
        result.push(char::from(b'A' + remainder));
        column = (column - 1) / 26;
    }
    result.iter().rev().collect()
}

pub(crate) fn a1(address: CellAddress) -> String {
    format!("{}{}", column_name(address.column), address.row)
}

pub(crate) fn quote_sheet_name(name: &str) -> String {
    format!("'{}'", name.replace('\'', "''"))
}

pub(crate) fn extract_formula_references(
    formula: &str,
    default_sheet: &str,
) -> Result<Vec<(String, CellRange)>, String> {
    let pattern = Regex::new(
        r"(?:(?:'((?:[^']|'')+)'|([A-Za-z_][A-Za-z0-9_.]*))!)?(\$?[A-Za-z]{1,3}\$?[0-9]{1,7})(?::(\$?[A-Za-z]{1,3}\$?[0-9]{1,7}))?",
    )
    .map_err(|error| error.to_string())?;
    pattern
        .captures_iter(formula)
        .filter_map(|captures| {
            let matched = captures.get(0).unwrap();
            if captures.get(1).is_none()
                && captures.get(2).is_none()
                && formula.as_bytes().get(matched.end()) == Some(&b'(')
            {
                return None;
            }
            let sheet = captures
                .get(1)
                .map(|value| value.as_str().replace("''", "'"))
                .or_else(|| captures.get(2).map(|value| value.as_str().to_string()))
                .unwrap_or_else(|| default_sheet.to_string());
            let start = captures.get(3).unwrap().as_str();
            let end = captures.get(4).map(|value| value.as_str()).unwrap_or(start);
            Some(
                (|| -> Result<CellRange, String> {
                    Ok(CellRange {
                        start: parse_cell_address(start)?,
                        end: parse_cell_address(end)?,
                    })
                })()
                .map(|range| (sheet, range)),
            )
        })
        .collect()
}

pub(crate) fn formula_is_external_or_active(formula: &str) -> bool {
    let uppercase = formula
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>()
        .to_ascii_uppercase();
    formula.contains('[')
        || formula.contains(']')
        || formula.contains('|')
        || uppercase.contains("WEBSERVICE(")
        || uppercase.contains("FILTERXML(")
        || uppercase.contains("RTD(")
        || uppercase.contains("DDE(")
        || uppercase.contains("CALL(")
        || uppercase.contains("EXEC(")
        || uppercase.contains("REGISTER.ID(")
        || uppercase.contains("EVALUATE(")
        || uppercase.contains("GET.CELL(")
        || uppercase.contains("GET.WORKSPACE(")
        || uppercase.contains("HYPERLINK(")
        || uppercase.contains("_XLL.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_excel_bounds_and_qualified_ranges() {
        assert_eq!(
            parse_cell_address("$XFD$1048576").unwrap(),
            CellAddress {
                row: MAX_ROWS,
                column: MAX_COLUMNS
            }
        );
        assert!(parse_cell_address("XFE1").is_err());
        let (sheet, range) = split_qualified_range("'Sales Q1'!B2:C4", "Other").unwrap();
        assert_eq!(sheet, "Sales Q1");
        assert_eq!(range.cell_count(), 6);
    }

    #[test]
    fn identifies_active_formula_functions() {
        assert!(formula_is_external_or_active("WEBSERVICE(A1)"));
        assert!(formula_is_external_or_active("[external.xlsx]Sheet1!A1"));
        assert!(formula_is_external_or_active("cmd|' /C calc'!A0"));
        assert!(formula_is_external_or_active("EVALUATE(A1)"));
        assert!(formula_is_external_or_active("GET.CELL(6,A1)"));
        assert!(!formula_is_external_or_active("SUM(A1:A5)"));
    }

    #[test]
    fn function_names_ending_in_digits_are_not_cell_references() {
        let references = extract_formula_references("LOG10(B2)", "Sheet1").unwrap();
        assert_eq!(references.len(), 1);
        assert_eq!(
            references[0].1,
            CellRange {
                start: CellAddress { row: 2, column: 2 },
                end: CellAddress { row: 2, column: 2 }
            }
        );
    }
}
