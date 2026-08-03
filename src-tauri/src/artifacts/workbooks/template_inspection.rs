use super::{
    address::{
        a1, formula_is_external_or_active, parse_cell_address, parse_local_range, CellAddress,
    },
    policy::enforce_safe_package,
    revision::{attribute, imported_sheet_part, verify_imported_structure},
    zip::read_zip,
    SheetVisibility, WorkbookTemplateSheet, WorksheetBounds,
};
use crate::foundation::digest::sha256_hex;
use regex::Regex;
use std::collections::{BTreeMap, HashSet};

pub(crate) fn inspect_template(bytes: &[u8]) -> Result<Vec<WorkbookTemplateSheet>, String> {
    let entries = safe_entries(bytes)?;
    inspect_entries(&entries)
}

fn safe_entries(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let entries = read_zip(bytes)?;
    reject_xlm_macro_sheets(&entries)?;
    enforce_safe_package(&entries)?;
    verify_imported_structure(&entries)?;
    if entries.contains_key("customXml/item1.xml") {
        return Err("OOMU-created workbooks must use their existing revision record.".to_string());
    }
    scan_imported_formulas(&entries)?;
    Ok(entries)
}

fn reject_xlm_macro_sheets(entries: &BTreeMap<String, Vec<u8>>) -> Result<(), String> {
    let macro_path = entries
        .keys()
        .any(|name| name.to_ascii_lowercase().contains("macrosheets/"));
    let macro_metadata = entries.iter().any(|(name, bytes)| {
        let name = name.to_ascii_lowercase();
        if name != "[content_types].xml" && !name.ends_with(".rels") {
            return false;
        }
        let xml = String::from_utf8_lossy(bytes).to_ascii_lowercase();
        xml.contains("application/vnd.ms-excel.macrosheet+xml")
            || (name.ends_with(".rels") && xml.contains("macrosheet"))
    });
    if macro_path || macro_metadata {
        return Err("Imported XLM macro sheets are unsupported.".to_string());
    }
    Ok(())
}

fn scan_imported_formulas(entries: &BTreeMap<String, Vec<u8>>) -> Result<(), String> {
    for (name, bytes) in entries {
        let lowercase = name.to_ascii_lowercase();
        let tags: &[&str] = if lowercase == "xl/workbook.xml" {
            &["definedName"]
        } else if lowercase.starts_with("xl/worksheets/") && lowercase.ends_with(".xml") {
            &["f", "formula", "formula1", "formula2"]
        } else if lowercase.starts_with("xl/tables/") && lowercase.ends_with(".xml") {
            &["calculatedColumnFormula", "totalsRowFormula"]
        } else if lowercase.starts_with("xl/charts/") && lowercase.ends_with(".xml") {
            &["f"]
        } else {
            continue;
        };
        let xml = std::str::from_utf8(bytes)
            .map_err(|_| "Imported formula XML is not UTF-8.".to_string())?;
        for tag in tags {
            for raw in tag_values(xml, tag)? {
                let formula = xml_unescape(raw)?;
                if formula_is_external_or_active(&formula) {
                    return Err(
                        "Imported workbook contains an external or active-content formula."
                            .to_string(),
                    );
                }
            }
        }
    }
    Ok(())
}

fn tag_values<'a>(xml: &'a str, tag: &str) -> Result<Vec<&'a str>, String> {
    let pattern = Regex::new(&format!(
        r"(?is)<(?:[A-Za-z_][A-Za-z0-9_.-]*:)?{tag}\b[^>]*>(.*?)</(?:[A-Za-z_][A-Za-z0-9_.-]*:)?{tag}\s*>"
    ))
    .map_err(|error| error.to_string())?;
    Ok(pattern
        .captures_iter(xml)
        .filter_map(|captures| captures.get(1).map(|value| value.as_str()))
        .collect())
}

fn inspect_entries(
    entries: &BTreeMap<String, Vec<u8>>,
) -> Result<Vec<WorkbookTemplateSheet>, String> {
    let workbook = std::str::from_utf8(
        entries
            .get("xl/workbook.xml")
            .ok_or_else(|| "Imported workbook XML is missing.".to_string())?,
    )
    .map_err(|_| "Imported workbook XML is not UTF-8.".to_string())?;
    let sheet_pattern = Regex::new(r"<sheet\b[^>]*>").map_err(|error| error.to_string())?;
    let mut names = HashSet::new();
    let mut result = Vec::new();
    for (index, tag) in sheet_pattern.find_iter(workbook).enumerate() {
        let name = attribute(tag.as_str(), "name")
            .ok_or_else(|| "Imported worksheet name is missing.".to_string())?;
        if !names.insert(name.to_lowercase()) {
            return Err("Imported worksheet names are duplicated.".to_string());
        }
        let state = match attribute(tag.as_str(), "state").as_deref() {
            Some("hidden") => SheetVisibility::Hidden,
            Some("veryHidden") => SheetVisibility::VeryHidden,
            Some(_) => return Err("Imported worksheet visibility is invalid.".to_string()),
            None => SheetVisibility::Visible,
        };
        let part = imported_sheet_part(entries, &name)?;
        let sheet_xml = std::str::from_utf8(
            entries
                .get(&part)
                .ok_or_else(|| "Imported worksheet part is missing.".to_string())?,
        )
        .map_err(|_| "Imported worksheet is not UTF-8 XML.".to_string())?;
        let bounds = sheet_bounds(sheet_xml)?;
        let formulas = formula_cells(sheet_xml)?;
        result.push(WorkbookTemplateSheet {
            sheet_id: format!(
                "template_sheet_{:04}_{}",
                index + 1,
                &sha256_hex(name.as_bytes())[..12]
            ),
            name,
            row_count: bounds.row_count,
            column_count: bounds.column_count,
            contains_formulas: !formulas.is_empty(),
            visibility: state,
        });
    }
    if result.is_empty() || result.len() > 1_024 {
        return Err("Imported workbook worksheet count is invalid.".to_string());
    }
    Ok(result)
}

fn sheet_bounds(xml: &str) -> Result<WorksheetBounds, String> {
    let dimension_pattern = Regex::new(r"<dimension\b[^>]*/?>").map_err(|e| e.to_string())?;
    if let Some(tag) = dimension_pattern.find(xml) {
        if let Some(raw) = attribute(tag.as_str(), "ref") {
            let range = parse_local_range(raw.split('!').next_back().unwrap_or(&raw))?;
            return Ok(WorksheetBounds {
                row_count: range.end.row.max(1),
                column_count: range.end.column.max(1),
            });
        }
    }
    let cell_pattern = Regex::new(r"<c\b[^>]*/?>").map_err(|e| e.to_string())?;
    let maximum = cell_pattern
        .find_iter(xml)
        .filter_map(|tag| attribute(tag.as_str(), "r"))
        .map(|raw| parse_cell_address(&raw))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .fold(CellAddress { row: 1, column: 1 }, |maximum, cell| {
            CellAddress {
                row: maximum.row.max(cell.row),
                column: maximum.column.max(cell.column),
            }
        });
    Ok(WorksheetBounds {
        row_count: maximum.row,
        column_count: maximum.column,
    })
}

fn formula_cells(xml: &str) -> Result<Vec<(String, String)>, String> {
    if Regex::new(r"<f\b[^>]*/>")
        .map_err(|error| error.to_string())?
        .is_match(xml)
    {
        return Err(
            "Imported shared formulas without explicit expressions are unsupported.".to_string(),
        );
    }
    let cell_pattern =
        Regex::new(r"(?s)<c\b([^>]*)>(.*?)</c>").map_err(|error| error.to_string())?;
    let formula_pattern =
        Regex::new(r"(?s)<f\b[^>]*>(.*?)</f>").map_err(|error| error.to_string())?;
    let mut formulas = Vec::new();
    for cell in cell_pattern.captures_iter(xml) {
        let Some(formula) = formula_pattern.captures(&cell[2]) else {
            continue;
        };
        let address = attribute(&format!("<c{}>", &cell[1]), "r")
            .ok_or_else(|| "Imported formula cell address is missing.".to_string())?;
        parse_cell_address(&address)?;
        let expression = xml_unescape(formula.get(1).unwrap().as_str())?;
        if expression.trim().is_empty() {
            return Err("Imported formula expression is empty.".to_string());
        }
        formulas.push((a1(parse_cell_address(&address)?), expression));
    }
    Ok(formulas)
}

fn xml_unescape(value: &str) -> Result<String, String> {
    if value.contains("&#") {
        return Err("Imported formula contains unsupported numeric XML entities.".to_string());
    }
    let mut remainder = value;
    while let Some(index) = remainder.find('&') {
        let entity = &remainder[index..];
        let Some(length) = ["&lt;", "&gt;", "&quot;", "&apos;", "&amp;"]
            .into_iter()
            .find(|allowed| entity.starts_with(allowed))
            .map(str::len)
        else {
            return Err("Imported formula contains an unsupported XML entity.".to_string());
        };
        remainder = &entity[length..];
    }
    let decoded = value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&");
    if decoded.contains("&#") {
        return Err("Imported formula contains unsupported numeric XML entities.".to_string());
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifacts::workbooks::{
        deterministic_fixture, ooxml::package_entries, zip::write_store_zip,
    };

    fn imported_fixture_entries() -> BTreeMap<String, Vec<u8>> {
        let mut entries = package_entries(&deterministic_fixture().unwrap()).unwrap();
        entries.remove("customXml/item1.xml");
        for (name, fragment) in [("_rels/.rels", "<Relationship Id=\"rId4\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml\" Target=\"customXml/item1.xml\"/>"), ("[Content_Types].xml", "<Override PartName=\"/customXml/item1.xml\" ContentType=\"application/vnd.oomu.workbook-ir+xml\"/>")] {
            let xml = String::from_utf8(entries[name].clone()).unwrap().replace(fragment, "");
            entries.insert(name.to_string(), xml.into_bytes());
        }
        entries
    }

    #[test]
    fn inspection_is_bounded_and_formula_truthful() {
        let entries = imported_fixture_entries();
        let bytes = write_store_zip(&entries).unwrap();
        let sheets = inspect_template(&bytes).unwrap();
        assert_eq!(sheets.len(), 2);
        assert!(sheets[0].contains_formulas);
    }

    #[test]
    fn inspection_rejects_active_formulas_in_cells_names_and_rules() {
        for (part, closing, injected) in [
            (
                "xl/worksheets/sheet1.xml",
                "</worksheet>",
                "<c r=\"Z1\"><f>WEBSERVICE (A1)</f></c>",
            ),
            (
                "xl/workbook.xml",
                "</workbook>",
                "<definedNames><definedName name=\"Unsafe\">HYPERLINK(A1)</definedName></definedNames>",
            ),
            (
                "xl/worksheets/sheet1.xml",
                "</worksheet>",
                "<dataValidations count=\"1\"><dataValidation sqref=\"A1\"><formula1>RTD (A1)</formula1></dataValidation></dataValidations>",
            ),
            (
                "xl/worksheets/sheet1.xml",
                "</worksheet>",
                "<conditionalFormatting sqref=\"A1\"><cfRule type=\"expression\"><formula>CALL (A1)</formula></cfRule></conditionalFormatting>",
            ),
            (
                "xl/tables/table1.xml",
                "</table>",
                "<tableColumn id=\"99\" name=\"Unsafe\"><calculatedColumnFormula>EXEC (A1)</calculatedColumnFormula></tableColumn>",
            ),
            (
                "xl/charts/chart1.xml",
                "</c:chartSpace>",
                "<c:f>[external.xlsx]Sheet1!A1</c:f>",
            ),
        ] {
            let mut entries = imported_fixture_entries();
            let xml = String::from_utf8(entries[part].clone())
                .unwrap()
                .replace(closing, &format!("{injected}{closing}"));
            entries.insert(part.to_string(), xml.into_bytes());
            let error = inspect_template(&write_store_zip(&entries).unwrap()).unwrap_err();
            assert!(error.contains("active-content"), "{part}: {error}");
        }
    }

    #[test]
    fn inspection_rejects_every_xlm_macro_sheet_marker() {
        for marker in 0..3 {
            let mut entries = imported_fixture_entries();
            match marker {
                0 => {
                    entries.insert(
                        "xl/macrosheets/sheet1.xml".to_string(),
                        b"<worksheet/>".to_vec(),
                    );
                }
                1 => {
                    let xml = String::from_utf8(entries["[Content_Types].xml"].clone())
                        .unwrap()
                        .replace(
                            "</Types>",
                            "<Override PartName=\"/xl/macrosheets/sheet1.xml\" ContentType=\"application/vnd.ms-excel.macrosheet+xml\"/></Types>",
                        );
                    entries.insert("[Content_Types].xml".to_string(), xml.into_bytes());
                }
                _ => {
                    let part = "xl/_rels/workbook.xml.rels";
                    let xml = String::from_utf8(entries[part].clone())
                        .unwrap()
                        .replace(
                            "</Relationships>",
                            "<Relationship Id=\"unsafe\" Type=\"http://schemas.microsoft.com/office/2006/relationships/xlMacrosheet\" Target=\"macrosheets/sheet1.xml\"/></Relationships>",
                        );
                    entries.insert(part.to_string(), xml.into_bytes());
                }
            }
            assert!(inspect_template(&write_store_zip(&entries).unwrap())
                .unwrap_err()
                .contains("XLM macro"));
        }
    }
}
