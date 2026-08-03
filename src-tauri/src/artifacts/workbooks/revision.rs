use super::{
    address::{
        a1, formula_is_external_or_active, parse_cell_address, parse_local_range, CellAddress,
        CellRange,
    },
    policy::enforce_safe_package,
    style_xml::{xml_attr, xml_text},
    validate_workbook,
    zip::{read_zip, write_store_zip},
    CellValue, RecalculationState, RecalculationStatus, WorkbookCell, WorkbookIr,
};
use crate::foundation::digest::sha256_hex;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fmt,
};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkbookRangeRevision {
    pub sheet_id: String,
    #[serde(default)]
    pub target_range: Option<String>,
    pub instruction: String,
    #[serde(default)]
    pub replacement_cells: Option<Vec<WorkbookCell>>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkbookRevisionErrorCode {
    WorkbookRevisionInstructionUnsupported,
    WorkbookRevisionTargetRequired,
    WorkbookRevisionTargetAmbiguous,
    WorkbookRevisionTargetMismatch,
    WorkbookRevisionUnsafePackage,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkbookRevisionError {
    pub code: WorkbookRevisionErrorCode,
    pub message: String,
}

impl fmt::Display for WorkbookRevisionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}
impl std::error::Error for WorkbookRevisionError {}

pub fn revise_range(
    base: &WorkbookIr,
    request: &WorkbookRangeRevision,
) -> Result<WorkbookIr, WorkbookRevisionError> {
    validate_workbook(base).map_err(unsupported)?;
    if request.instruction.trim().is_empty() || request.instruction.chars().count() > 2_000 {
        return Err(unsupported("A concise revision instruction is required."));
    }
    let sheet_index = base
        .worksheets
        .iter()
        .position(|sheet| sheet.sheet_id == request.sheet_id)
        .ok_or_else(|| mismatch("Selected worksheet was not found."))?;
    let mut revised = base.clone();
    let target = resolve_target(&revised.worksheets[sheet_index], request)?;
    let value_changed = if let Some(replacements) = &request.replacement_cells {
        apply_exact_replacements(&mut revised.worksheets[sheet_index], target, replacements)?
    } else {
        apply_instruction(
            &mut revised.worksheets[sheet_index],
            target,
            &request.instruction,
        )?
    };
    revised.revision = revised
        .revision
        .checked_add(1)
        .ok_or_else(|| unsupported("Workbook revision overflow."))?;
    if value_changed {
        for cell in revised
            .worksheets
            .iter_mut()
            .flat_map(|sheet| &mut sheet.cells)
        {
            if let CellValue::Formula { cached_value, .. } = &mut cell.value {
                *cached_value = None;
                cell.provenance.clear();
            }
        }
        let has_formulas = revised
            .worksheets
            .iter()
            .flat_map(|sheet| &sheet.cells)
            .any(|cell| matches!(cell.value, CellValue::Formula { .. }));
        revised.recalculation = if has_formulas {
            RecalculationState {
                status: RecalculationStatus::Stale,
                ..RecalculationState::default()
            }
        } else {
            RecalculationState::default()
        };
        if has_formulas {
            let _ = super::recalculate_supported_formulas(&mut revised);
        }
    }
    validate_workbook(&revised).map_err(unsupported)?;
    Ok(revised)
}

fn resolve_target(
    sheet: &super::Worksheet,
    request: &WorkbookRangeRevision,
) -> Result<CellRange, WorkbookRevisionError> {
    if let Some(target) = &request.target_range {
        return parse_local_range(target).map_err(mismatch);
    }
    if let Some(replacements) = &request.replacement_cells {
        if replacements.is_empty() {
            return Err(required("Exact replacement cells cannot be empty."));
        }
        return bounding_range(replacements).map_err(mismatch);
    }
    if let Some(range) = explicit_instruction_range(&request.instruction) {
        return parse_local_range(&range).map_err(mismatch);
    }
    if let Some((old, _)) = parse_replace(&request.instruction) {
        let matches = sheet
            .cells
            .iter()
            .filter(|cell| matches!(&cell.value, CellValue::Text { value } if value.contains(&old)))
            .collect::<Vec<_>>();
        return match matches.as_slice() {
            [] => Err(mismatch(
                "Revision text was not found on the selected worksheet.",
            )),
            [cell] => parse_local_range(&cell.address).map_err(mismatch),
            _ => Err(ambiguous(
                "Revision text appears in more than one cell; choose a target.",
            )),
        };
    }
    Err(required(
        "Choose a target or include an explicit A1 range in the instruction.",
    ))
}

fn apply_exact_replacements(
    sheet: &mut super::Worksheet,
    target: CellRange,
    replacements: &[WorkbookCell],
) -> Result<bool, WorkbookRevisionError> {
    if replacements.is_empty() || replacements.len() > 10_000 {
        return Err(mismatch("Exact replacements require 1 to 10,000 cells."));
    }
    let mut seen = HashSet::new();
    for replacement in replacements {
        let address = parse_cell_address(&replacement.address).map_err(mismatch)?;
        if !target.contains(address) || !seen.insert((address.row, address.column)) {
            return Err(mismatch(
                "A replacement is outside the target or duplicated.",
            ));
        }
        let mut sanitized = replacement.clone();
        sanitized.provenance.clear();
        if let Some(existing) = sheet.cells.iter_mut().find(|cell| {
            cell.address
                .replace('$', "")
                .eq_ignore_ascii_case(&a1(address))
        }) {
            *existing = sanitized;
        } else {
            sheet.cells.push(sanitized);
        }
    }
    Ok(true)
}

fn apply_instruction(
    sheet: &mut super::Worksheet,
    target: CellRange,
    instruction: &str,
) -> Result<bool, WorkbookRevisionError> {
    let trimmed = instruction.trim();
    if let Some((old, new)) = parse_replace(trimmed) {
        let mut changed = 0;
        for cell in &mut sheet.cells {
            let address = parse_cell_address(&cell.address).map_err(mismatch)?;
            if target.contains(address) {
                if let CellValue::Text { value } = &mut cell.value {
                    if value.contains(&old) {
                        *value = value.replace(&old, &new);
                        cell.provenance.clear();
                        changed += 1;
                    }
                }
            }
        }
        if changed == 0 {
            return Err(mismatch("Revision text was not found in the target."));
        }
        return Ok(true);
    }
    let normalized = remove_explicit_range(trimmed);
    if normalized.eq_ignore_ascii_case("clear selected cells")
        || normalized.eq_ignore_ascii_case("clear these cells")
        || normalized.eq_ignore_ascii_case("clear")
    {
        for address in addresses(target)? {
            set_cell_value(sheet, address, CellValue::Blank);
        }
        return Ok(true);
    }
    let format_pattern = Regex::new(r"(?i)^format(?: selected cells)? as ([A-Za-z0-9_.-]{1,256})$")
        .map_err(|error| unsupported(error.to_string()))?;
    if let Some(captures) = format_pattern.captures(&normalized) {
        let format_id = captures.get(1).unwrap().as_str();
        if !sheet.cells.iter().any(|cell| {
            target.contains(
                parse_cell_address(&cell.address).unwrap_or(CellAddress { row: 0, column: 0 }),
            )
        }) {
            return Err(mismatch("Formatting requires existing target cells."));
        }
        for cell in &mut sheet.cells {
            if target.contains(parse_cell_address(&cell.address).map_err(mismatch)?) {
                cell.format_id = Some(format_id.to_string());
            }
        }
        return Ok(false);
    }
    let set = Regex::new(r"(?is)^set(?: selected cells)? to (text|number|date|formula):\s*(.+)$")
        .map_err(|error| unsupported(error.to_string()))?;
    let value = if let Some(captures) = set.captures(&normalized) {
        let kind = captures.get(1).unwrap().as_str().to_ascii_lowercase();
        let raw = captures.get(2).unwrap().as_str().trim();
        match kind.as_str() {
            "text" => CellValue::Text {
                value: raw.to_string(),
            },
            "number" => CellValue::Number {
                value: raw
                    .parse::<f64>()
                    .map_err(|_| unsupported("Number revision value is invalid."))?,
            },
            "date" => CellValue::Date {
                iso: raw.to_string(),
            },
            "formula" if !formula_is_external_or_active(raw) => CellValue::Formula {
                expression: raw.strip_prefix('=').unwrap_or(raw).to_string(),
                cached_value: None,
            },
            "formula" => return Err(unsupported("External or active formulas are forbidden.")),
            _ => return Err(unsupported("Revision value type is unsupported.")),
        }
    } else {
        let obvious =
            Regex::new(r"(?is)^(?:set|change)(?: selected cells| these cells)? to\s+(.+)$")
                .map_err(|error| unsupported(error.to_string()))?;
        let raw = obvious
            .captures(&normalized)
            .and_then(|captures| captures.get(1))
            .map(|value| value.as_str().trim())
            .ok_or_else(|| {
                unsupported(
                    "The requested change is ambiguous; choose cells and state the new value.",
                )
            })?;
        infer_plain_value(raw)?
    };
    for address in addresses(target)? {
        set_cell_value(sheet, address, value.clone());
    }
    Ok(true)
}

fn explicit_instruction_range(instruction: &str) -> Option<String> {
    let pattern = Regex::new(r"(?i)^(?:set|change|clear|format)\s+(\$?[A-Z]{1,3}\$?[1-9][0-9]{0,6}(?::\$?[A-Z]{1,3}\$?[1-9][0-9]{0,6})?)\b").ok()?;
    pattern
        .captures(instruction.trim())
        .and_then(|captures| captures.get(1).map(|value| value.as_str().to_string()))
}

fn remove_explicit_range(instruction: &str) -> String {
    let pattern = Regex::new(r"(?i)^(set|change|clear|format)\s+\$?[A-Z]{1,3}\$?[1-9][0-9]{0,6}(?::\$?[A-Z]{1,3}\$?[1-9][0-9]{0,6})?\s*").expect("static range regex");
    pattern.replace(instruction, "$1 ").trim().to_string()
}

fn infer_plain_value(raw: &str) -> Result<CellValue, WorkbookRevisionError> {
    if raw.starts_with('=') || raw.to_ascii_lowercase().starts_with("formula:") {
        return Err(unsupported(
            "Formula changes must be stated explicitly as a formula.",
        ));
    }
    if let Ok(number) = raw.parse::<f64>() {
        if number.is_finite() {
            return Ok(CellValue::Number { value: number });
        }
    }
    if chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d").is_ok()
        || chrono::DateTime::parse_from_rfc3339(raw).is_ok()
    {
        return Ok(CellValue::Date {
            iso: raw.to_string(),
        });
    }
    Ok(CellValue::Text {
        value: raw.to_string(),
    })
}

fn parse_replace(instruction: &str) -> Option<(String, String)> {
    let pattern =
        Regex::new(r#"(?is)^replace(?: text)?\s+[\"“](.+?)[\"”]\s+with\s+[\"“](.+?)[\"”]$"#)
            .ok()?;
    let captures = pattern.captures(instruction.trim())?;
    Some((
        captures.get(1)?.as_str().to_string(),
        captures.get(2)?.as_str().to_string(),
    ))
}

fn addresses(range: CellRange) -> Result<Vec<CellAddress>, WorkbookRevisionError> {
    if range.cell_count() > 10_000 {
        return Err(mismatch("Revision target exceeds 10,000 cells."));
    }
    Ok((range.start.row..=range.end.row)
        .flat_map(|row| {
            (range.start.column..=range.end.column).map(move |column| CellAddress { row, column })
        })
        .collect())
}

fn set_cell_value(sheet: &mut super::Worksheet, address: CellAddress, value: CellValue) {
    let label = a1(address);
    if let Some(cell) = sheet
        .cells
        .iter_mut()
        .find(|cell| cell.address.replace('$', "").eq_ignore_ascii_case(&label))
    {
        cell.value = value;
        cell.provenance.clear();
    } else {
        sheet.cells.push(WorkbookCell {
            address: label,
            value,
            format_id: None,
            comment: None,
            provenance: vec![],
        });
    }
}

fn bounding_range(cells: &[WorkbookCell]) -> Result<CellRange, String> {
    let parsed = cells
        .iter()
        .map(|cell| parse_cell_address(&cell.address))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CellRange {
        start: CellAddress {
            row: parsed.iter().map(|value| value.row).min().unwrap(),
            column: parsed.iter().map(|value| value.column).min().unwrap(),
        },
        end: CellAddress {
            row: parsed.iter().map(|value| value.row).max().unwrap(),
            column: parsed.iter().map(|value| value.column).max().unwrap(),
        },
    })
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedPackageRevision {
    #[serde(skip)]
    pub bytes: Vec<u8>,
    pub sha256: String,
    pub target_part: String,
    pub changed_parts: Vec<String>,
    pub preserved_part_digests: BTreeMap<String, String>,
}

pub fn revise_imported_xlsx(
    bytes: &[u8],
    sheet_name: &str,
    request: &WorkbookRangeRevision,
) -> Result<ImportedPackageRevision, WorkbookRevisionError> {
    let original = read_zip(bytes).map_err(unsafe_package)?;
    enforce_safe_package(&original).map_err(unsafe_package)?;
    verify_imported_structure(&original).map_err(unsafe_package)?;
    if original.contains_key("customXml/item1.xml") {
        return Err(unsupported(
            "OOMU-created workbooks must be revised through their typed revision record.",
        ));
    }
    let replacements = request
        .replacement_cells
        .as_ref()
        .filter(|cells| !cells.is_empty())
        .ok_or_else(|| {
            unsupported("Imported workbook revisions require exact replacement cells.")
        })?;
    let target = request
        .target_range
        .as_ref()
        .map(|value| parse_local_range(value))
        .transpose()
        .map_err(mismatch)?
        .unwrap_or(bounding_range(replacements).map_err(mismatch)?);
    let target_part = imported_sheet_part(&original, sheet_name).map_err(unsafe_package)?;
    let mut revised = original.clone();
    let sheet_bytes = revised
        .get(&target_part)
        .ok_or_else(|| unsafe_package("Imported worksheet part is missing."))?;
    let mut sheet_xml = std::str::from_utf8(sheet_bytes)
        .map_err(|_| unsafe_package("Imported worksheet is not UTF-8 XML."))?
        .to_string();
    let mut spans = Vec::new();
    let mut replacement_addresses = HashSet::new();
    for replacement in replacements {
        let address = parse_cell_address(&replacement.address).map_err(mismatch)?;
        if !target.contains(address)
            || !replacement_addresses.insert((address.row, address.column))
            || replacement.comment.is_some()
            || !replacement.provenance.is_empty()
            || replacement.format_id.is_some()
        {
            return Err(mismatch("Imported replacement is outside the target or attempts an unsupported metadata/style edit."));
        }
        let (start, end, start_tag) =
            find_existing_cell(&sheet_xml, &a1(address)).map_err(mismatch)?;
        spans.push((start, end, imported_cell_xml(replacement, &start_tag)?));
    }
    spans.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
    for (start, end, replacement) in spans {
        sheet_xml.replace_range(start..end, &replacement);
    }
    revised.insert(target_part.clone(), sheet_xml.into_bytes());
    let mut changed_parts = vec![target_part.clone()];
    let workbook = std::str::from_utf8(
        revised
            .get("xl/workbook.xml")
            .ok_or_else(|| unsafe_package("Imported workbook XML is missing."))?,
    )
    .map_err(|_| unsafe_package("Imported workbook XML is not UTF-8."))?;
    revised.insert(
        "xl/workbook.xml".to_string(),
        mark_calculation_stale(workbook).into_bytes(),
    );
    changed_parts.push("xl/workbook.xml".to_string());
    enforce_safe_package(&revised).map_err(unsafe_package)?;
    verify_imported_structure(&revised).map_err(unsafe_package)?;
    let preserved_part_digests = original
        .iter()
        .filter(|(name, _)| !changed_parts.contains(name))
        .map(|(name, data)| {
            if revised.get(name) != Some(data) {
                return Err(unsafe_package(format!(
                    "Unrelated workbook part {name} changed during revision."
                )));
            }
            Ok((name.clone(), sha256_hex(data)))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let bytes = write_store_zip(&revised).map_err(unsafe_package)?;
    Ok(ImportedPackageRevision {
        sha256: sha256_hex(&bytes),
        bytes,
        target_part,
        changed_parts,
        preserved_part_digests,
    })
}

pub(super) fn verify_imported_structure(entries: &BTreeMap<String, Vec<u8>>) -> Result<(), String> {
    for required in [
        "[Content_Types].xml",
        "_rels/.rels",
        "xl/workbook.xml",
        "xl/_rels/workbook.xml.rels",
        "xl/styles.xml",
    ] {
        if !entries.contains_key(required) {
            return Err(format!(
                "Imported workbook is missing required part {required}."
            ));
        }
    }
    super::verification::verify_relationship_targets(entries)
}

pub(super) fn imported_sheet_part(
    entries: &BTreeMap<String, Vec<u8>>,
    sheet_name: &str,
) -> Result<String, String> {
    let workbook = std::str::from_utf8(
        entries
            .get("xl/workbook.xml")
            .ok_or_else(|| "Imported workbook XML is missing.".to_string())?,
    )
    .map_err(|_| "Imported workbook XML is not UTF-8.".to_string())?;
    let sheet_tag = Regex::new(r"<sheet\b[^>]*>")
        .map_err(|error| error.to_string())?
        .find_iter(workbook)
        .find(|tag| attribute(tag.as_str(), "name").as_deref() == Some(sheet_name))
        .ok_or_else(|| "Imported worksheet name was not found.".to_string())?;
    let relationship_id = attribute(sheet_tag.as_str(), "r:id")
        .ok_or_else(|| "Imported worksheet relationship is missing.".to_string())?;
    let rels = std::str::from_utf8(
        entries
            .get("xl/_rels/workbook.xml.rels")
            .ok_or_else(|| "Imported workbook relationships are missing.".to_string())?,
    )
    .map_err(|_| "Imported workbook relationships are not UTF-8.".to_string())?;
    let relationship = Regex::new(r"<Relationship\b[^>]*/?>")
        .map_err(|error| error.to_string())?
        .find_iter(rels)
        .find(|tag| attribute(tag.as_str(), "Id").as_deref() == Some(&relationship_id))
        .ok_or_else(|| "Imported worksheet relationship target is missing.".to_string())?;
    let relationship_type = attribute(relationship.as_str(), "Type")
        .ok_or_else(|| "Imported worksheet relationship type is missing.".to_string())?;
    if !relationship_type.ends_with("/worksheet") {
        return Err("Imported worksheet relationship type is invalid.".to_string());
    }
    let target = attribute(relationship.as_str(), "Target")
        .ok_or_else(|| "Imported worksheet relationship target is missing.".to_string())?;
    resolve_xl_target(&target)
}

fn resolve_xl_target(target: &str) -> Result<String, String> {
    if target.starts_with('/') || target.contains('\\') {
        return Err("Imported worksheet target is unsafe.".to_string());
    }
    let mut parts = vec!["xl"];
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.len() <= 1 {
                    return Err("Imported worksheet target escapes package root.".to_string());
                }
                parts.pop();
            }
            value => parts.push(value),
        }
    }
    Ok(parts.join("/"))
}

fn find_existing_cell(xml: &str, address: &str) -> Result<(usize, usize, String), String> {
    let mut cursor = 0;
    let mut found = None;
    while let Some(relative) = xml[cursor..].find("<c") {
        let start = cursor + relative;
        let boundary = xml.as_bytes().get(start + 2).copied().unwrap_or_default();
        if !boundary.is_ascii_whitespace() && boundary != b'>' && boundary != b'/' {
            cursor = start + 2;
            continue;
        }
        let tag_end = scan_tag_end(xml, start)?;
        let tag = &xml[start..=tag_end];
        if attribute(tag, "r")
            .map(|value| value.replace('$', "").eq_ignore_ascii_case(address))
            .unwrap_or(false)
        {
            let end = if tag.trim_end().ends_with("/>") {
                tag_end + 1
            } else {
                xml[tag_end + 1..]
                    .find("</c>")
                    .map(|offset| tag_end + 1 + offset + 4)
                    .ok_or_else(|| "Imported cell closing tag is missing.".to_string())?
            };
            if found.is_some() {
                return Err(format!(
                    "Imported worksheet contains duplicate cell {address}."
                ));
            }
            found = Some((start, end, tag.to_string()));
        }
        cursor = tag_end + 1;
    }
    found.ok_or_else(|| format!("Imported revision target cell {address} does not already exist."))
}

fn scan_tag_end(xml: &str, start: usize) -> Result<usize, String> {
    let mut quote = None;
    for (offset, character) in xml[start..].char_indices() {
        if matches!(character, '\'' | '"') {
            if quote == Some(character) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(character);
            }
        }
        if character == '>' && quote.is_none() {
            return Ok(start + offset);
        }
    }
    Err("Imported worksheet contains a truncated cell tag.".to_string())
}

fn imported_cell_xml(
    cell: &WorkbookCell,
    start_tag: &str,
) -> Result<String, WorkbookRevisionError> {
    let style = attribute(start_tag, "s")
        .map(|value| format!(" s=\"{}\"", xml_attr(&value)))
        .unwrap_or_default();
    let address = xml_attr(&cell.address.replace('$', "").to_ascii_uppercase());
    let (kind, body) = match &cell.value {
        CellValue::Blank => ("", String::new()),
        CellValue::Text { value } => (" t=\"inlineStr\"", format!("<is><t xml:space=\"preserve\">{}</t></is>", xml_text(value))),
        CellValue::Number { value } if value.is_finite() => ("", format!("<v>{value}</v>")),
        CellValue::Boolean { value } => (" t=\"b\"", format!("<v>{}</v>", u8::from(*value))),
        CellValue::Formula { expression, .. } if !formula_is_external_or_active(expression) => ("", format!("<f>{}</f>", xml_text(expression.strip_prefix('=').unwrap_or(expression)))),
        CellValue::Date { .. } => return Err(unsupported("Imported date edits require an explicit serial number to preserve the template date system and style.")),
        _ => return Err(unsupported("Imported replacement value is invalid or unsafe.")),
    };
    Ok(format!("<c r=\"{address}\"{style}{kind}>{body}</c>"))
}

fn mark_calculation_stale(xml: &str) -> String {
    let replacement =
        "<calcPr calcId=\"191029\" calcMode=\"auto\" fullCalcOnLoad=\"1\" forceFullCalc=\"1\"/>";
    let pattern = Regex::new(r"<calcPr\b[^>]*/>").expect("static calcPr regex");
    if pattern.is_match(xml) {
        pattern.replace(xml, replacement).to_string()
    } else {
        xml.replacen("</workbook>", &format!("{replacement}</workbook>"), 1)
    }
}

pub(super) fn attribute(xml: &str, name: &str) -> Option<String> {
    let pattern = Regex::new(&format!(
        r#"\b{}\s*=\s*[\"']([^\"']*)[\"']"#,
        regex::escape(name)
    ))
    .ok()?;
    pattern
        .captures(xml)
        .and_then(|captures| captures.get(1).map(|value| value.as_str().to_string()))
}

fn unsupported(message: impl Into<String>) -> WorkbookRevisionError {
    WorkbookRevisionError {
        code: WorkbookRevisionErrorCode::WorkbookRevisionInstructionUnsupported,
        message: message.into(),
    }
}
fn required(message: impl Into<String>) -> WorkbookRevisionError {
    WorkbookRevisionError {
        code: WorkbookRevisionErrorCode::WorkbookRevisionTargetRequired,
        message: message.into(),
    }
}
fn ambiguous(message: impl Into<String>) -> WorkbookRevisionError {
    WorkbookRevisionError {
        code: WorkbookRevisionErrorCode::WorkbookRevisionTargetAmbiguous,
        message: message.into(),
    }
}
fn mismatch(message: impl Into<String>) -> WorkbookRevisionError {
    WorkbookRevisionError {
        code: WorkbookRevisionErrorCode::WorkbookRevisionTargetMismatch,
        message: message.into(),
    }
}
fn unsafe_package(message: impl Into<String>) -> WorkbookRevisionError {
    WorkbookRevisionError {
        code: WorkbookRevisionErrorCode::WorkbookRevisionUnsafePackage,
        message: message.into(),
    }
}

#[cfg(test)]
#[path = "revision_tests.rs"]
mod tests;
