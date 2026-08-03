use super::{
    policy::enforce_safe_package,
    sheet_xml::{build_sheet_parts, workbook_cell_index},
    style_xml::{build_styles, xml_attr, xml_text},
    validate_workbook,
    verification::{verify_and_render, WorkbookVerification},
    zip::write_store_zip,
    RecalculationStatus, SheetVisibility, WorkbookIr, WorkbookPreviewImage,
};
use crate::foundation::digest::sha256_hex;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct WorkbookBuildOutput {
    pub workbook: WorkbookIr,
    pub bytes: Vec<u8>,
    pub sha256: String,
    pub verification: WorkbookVerification,
    pub previews: Vec<WorkbookPreviewImage>,
}

pub fn build_workbook(workbook: &WorkbookIr) -> Result<WorkbookBuildOutput, String> {
    let mut normalized = workbook.clone();
    normalize_formula_state(&mut normalized)?;
    let validated = validate_workbook(&normalized)?;
    let entries = package_entries(&validated.0)?;
    enforce_safe_package(&entries)?;
    let bytes = write_store_zip(&entries)?;
    let (verification, previews) = verify_and_render(&bytes)?;
    Ok(WorkbookBuildOutput {
        workbook: validated.0,
        sha256: sha256_hex(&bytes),
        bytes,
        verification,
        previews,
    })
}

fn normalize_formula_state(workbook: &mut WorkbookIr) -> Result<(), String> {
    let has_formulas = workbook
        .worksheets
        .iter()
        .flat_map(|sheet| &sheet.cells)
        .any(|cell| matches!(cell.value, super::CellValue::Formula { .. }));
    if !has_formulas {
        workbook.recalculation = super::RecalculationState::default();
        return Ok(());
    }
    let mut qualified = workbook.clone();
    for cell in qualified
        .worksheets
        .iter_mut()
        .flat_map(|sheet| &mut sheet.cells)
    {
        if let super::CellValue::Formula { cached_value, .. } = &mut cell.value {
            *cached_value = None;
        }
    }
    qualified.recalculation = super::RecalculationState {
        status: RecalculationStatus::Stale,
        ..super::RecalculationState::default()
    };
    if super::recalculate_supported_formulas(&mut qualified).is_ok() {
        *workbook = qualified;
    } else {
        for cell in workbook
            .worksheets
            .iter_mut()
            .flat_map(|sheet| &mut sheet.cells)
        {
            if let super::CellValue::Formula { cached_value, .. } = &mut cell.value {
                *cached_value = None;
            }
        }
        workbook.recalculation = super::RecalculationState {
            status: RecalculationStatus::Stale,
            ..super::RecalculationState::default()
        };
    }
    Ok(())
}

pub(crate) fn package_entries(workbook: &WorkbookIr) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let styles = build_styles(workbook)?;
    let mut entries = BTreeMap::new();
    let mut overrides = vec![
        (
            "/xl/workbook.xml".to_string(),
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"
                .to_string(),
        ),
        (
            "/xl/styles.xml".to_string(),
            "application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml".to_string(),
        ),
        (
            "/docProps/core.xml".to_string(),
            "application/vnd.openxmlformats-package.core-properties+xml".to_string(),
        ),
        (
            "/docProps/app.xml".to_string(),
            "application/vnd.openxmlformats-officedocument.extended-properties+xml".to_string(),
        ),
        (
            "/customXml/item1.xml".to_string(),
            "application/vnd.oomu.workbook-ir+xml".to_string(),
        ),
    ];
    entries.insert("_rels/.rels".to_string(), root_relationships().into_bytes());
    entries.insert(
        "docProps/core.xml".to_string(),
        core_properties(workbook).into_bytes(),
    );
    entries.insert(
        "docProps/app.xml".to_string(),
        app_properties(workbook).into_bytes(),
    );
    entries.insert("xl/styles.xml".to_string(), styles.xml.clone());
    entries.insert(
        "xl/workbook.xml".to_string(),
        workbook_xml(workbook).into_bytes(),
    );
    entries.insert(
        "xl/_rels/workbook.xml.rels".to_string(),
        workbook_relationships(workbook).into_bytes(),
    );
    let mut next_table_id = 1_u32;
    let mut next_chart_id = 1_u32;
    let cell_index = workbook_cell_index(workbook)?;
    for (index, sheet) in workbook.worksheets.iter().enumerate() {
        let parts = build_sheet_parts(
            workbook,
            sheet,
            index,
            &styles,
            &mut next_table_id,
            &mut next_chart_id,
            &cell_index,
        )?;
        let sheet_path = format!("xl/worksheets/sheet{}.xml", index + 1);
        entries.insert(sheet_path.clone(), parts.sheet_xml);
        overrides.push((
            format!("/{sheet_path}"),
            "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml".to_string(),
        ));
        if let Some(relationships) = parts.relationships {
            entries.insert(
                format!("xl/worksheets/_rels/sheet{}.xml.rels", index + 1),
                relationships,
            );
        }
        entries.extend(parts.extra_parts);
        overrides.extend(parts.overrides);
    }
    let custom = embedded_ir_xml(workbook)?;
    entries.insert("customXml/item1.xml".to_string(), custom);
    entries.insert(
        "[Content_Types].xml".to_string(),
        content_types(&overrides).into_bytes(),
    );
    Ok(entries)
}

pub(crate) fn extract_embedded_ir(
    entries: &BTreeMap<String, Vec<u8>>,
) -> Result<WorkbookIr, String> {
    let xml = std::str::from_utf8(
        entries
            .get("customXml/item1.xml")
            .ok_or_else(|| "Workbook does not contain an OOMU workbook contract.".to_string())?,
    )
    .map_err(|_| "Embedded workbook contract is not UTF-8.".to_string())?;
    let root_start = xml
        .find("<oomuWorkbookIr")
        .ok_or_else(|| "Embedded workbook contract root is missing.".to_string())?;
    let after_name = xml
        .as_bytes()
        .get(root_start + "<oomuWorkbookIr".len())
        .copied()
        .unwrap_or_default();
    if !after_name.is_ascii_whitespace() && after_name != b'>' {
        return Err("Embedded workbook contract root is malformed.".to_string());
    }
    if xml[root_start + 1..].contains("<oomuWorkbookIr") {
        return Err("Embedded workbook contract must contain exactly one root.".to_string());
    }
    let prefix = xml[..root_start].trim();
    if !prefix.is_empty()
        && !(prefix.starts_with("<?xml")
            && prefix.ends_with("?>")
            && prefix[5..prefix.len() - 2].find("<?xml").is_none())
    {
        return Err("Embedded workbook contract contains content before its root.".to_string());
    }
    let start_tag_end = scan_root_tag_end(xml, root_start)? + 1;
    let closing = "</oomuWorkbookIr>";
    let end = xml[start_tag_end..]
        .find(closing)
        .map(|offset| start_tag_end + offset)
        .ok_or_else(|| "Embedded workbook contract closing tag is missing.".to_string())?;
    if xml.matches(closing).count() != 1 || !xml[end + closing.len()..].trim().is_empty() {
        return Err(
            "Embedded workbook contract contains trailing or multiple root content.".to_string(),
        );
    }
    let root_tag = &xml[root_start..start_tag_end];
    let encoded = xml[start_tag_end..end].trim();
    let decoded = STANDARD
        .decode(encoded)
        .map_err(|_| "Embedded workbook contract is not valid base64.".to_string())?;
    let expected = attribute(root_tag, "sha256")
        .ok_or_else(|| "Embedded workbook contract has no digest.".to_string())?;
    if sha256_hex(&decoded) != expected {
        return Err("Embedded workbook contract digest failed verification.".to_string());
    }
    let workbook: WorkbookIr = serde_json::from_slice(&decoded)
        .map_err(|error| format!("Embedded workbook contract is invalid: {error}"))?;
    validate_workbook(&workbook)?;
    Ok(workbook)
}

fn scan_root_tag_end(xml: &str, start: usize) -> Result<usize, String> {
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
    Err("Embedded workbook contract root tag is truncated.".to_string())
}

fn embedded_ir_xml(workbook: &WorkbookIr) -> Result<Vec<u8>, String> {
    let bytes = serde_json::to_vec(workbook).map_err(|error| error.to_string())?;
    Ok(format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><oomuWorkbookIr xmlns=\"urn:oomu:workbook-ir:1\" encoding=\"base64\" sha256=\"{}\">{}</oomuWorkbookIr>", sha256_hex(&bytes), STANDARD.encode(&bytes)).into_bytes())
}

fn content_types(overrides: &[(String, String)]) -> String {
    let override_xml = overrides
        .iter()
        .map(|(part, content_type)| {
            format!(
                "<Override PartName=\"{}\" ContentType=\"{}\"/>",
                xml_attr(part),
                xml_attr(content_type)
            )
        })
        .collect::<String>();
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"vml\" ContentType=\"application/vnd.openxmlformats-officedocument.vmlDrawing\"/>{override_xml}</Types>")
}

fn root_relationships() -> String {
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/><Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties\" Target=\"docProps/app.xml\"/><Relationship Id=\"rId4\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml\" Target=\"customXml/item1.xml\"/></Relationships>".to_string()
}

fn core_properties(workbook: &WorkbookIr) -> String {
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"><dc:title>{}</dc:title><dc:creator>OOMU</dc:creator><cp:lastModifiedBy>OOMU</cp:lastModifiedBy><dcterms:created xsi:type=\"dcterms:W3CDTF\">2000-01-01T00:00:00Z</dcterms:created><dcterms:modified xsi:type=\"dcterms:W3CDTF\">2000-01-01T00:00:00Z</dcterms:modified><cp:revision>{}</cp:revision></cp:coreProperties>", xml_text(&workbook.title), workbook.revision)
}

fn app_properties(workbook: &WorkbookIr) -> String {
    let titles = workbook
        .worksheets
        .iter()
        .map(|sheet| format!("<vt:lpstr>{}</vt:lpstr>", xml_text(&sheet.name)))
        .collect::<String>();
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\" xmlns:vt=\"http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes\"><Application>OOMU</Application><AppVersion>1.0</AppVersion><HeadingPairs><vt:vector size=\"2\" baseType=\"variant\"><vt:variant><vt:lpstr>Worksheets</vt:lpstr></vt:variant><vt:variant><vt:i4>{}</vt:i4></vt:variant></vt:vector></HeadingPairs><TitlesOfParts><vt:vector size=\"{}\" baseType=\"lpstr\">{titles}</vt:vector></TitlesOfParts></Properties>", workbook.worksheets.len(), workbook.worksheets.len())
}

fn workbook_xml(workbook: &WorkbookIr) -> String {
    let sheets = workbook
        .worksheets
        .iter()
        .enumerate()
        .map(|(index, sheet)| {
            let state = match sheet.visibility {
                SheetVisibility::Visible => "",
                SheetVisibility::Hidden => " state=\"hidden\"",
                SheetVisibility::VeryHidden => " state=\"veryHidden\"",
            };
            format!(
                "<sheet name=\"{}\" sheetId=\"{}\"{state} r:id=\"rId{}\"/>",
                xml_attr(&sheet.name),
                index + 1,
                index + 1
            )
        })
        .collect::<String>();
    let names = if workbook.named_ranges.is_empty() {
        String::new()
    } else {
        format!(
            "<definedNames>{}</definedNames>",
            workbook
                .named_ranges
                .iter()
                .map(|range| format!(
                    "<definedName name=\"{}\"{}>{}</definedName>",
                    xml_attr(&range.name),
                    range
                        .comment
                        .as_ref()
                        .map(|comment| format!(" comment=\"{}\"", xml_attr(comment)))
                        .unwrap_or_default(),
                    xml_text(range.formula.strip_prefix('=').unwrap_or(&range.formula))
                ))
                .collect::<String>()
        )
    };
    let calc = match workbook.recalculation.status {
        RecalculationStatus::Stale => {
            "<calcPr calcId=\"191029\" calcMode=\"auto\" fullCalcOnLoad=\"1\" forceFullCalc=\"1\"/>"
        }
        _ => {
            "<calcPr calcId=\"191029\" calcMode=\"auto\" fullCalcOnLoad=\"0\" forceFullCalc=\"0\"/>"
        }
    };
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><fileVersion appName=\"xl\" lastEdited=\"7\" lowestEdited=\"7\" rupBuild=\"27328\"/><workbookPr date1904=\"{}\"/><bookViews><workbookView xWindow=\"0\" yWindow=\"0\" windowWidth=\"24000\" windowHeight=\"12000\"/></bookViews><sheets>{sheets}</sheets>{names}{calc}</workbook>", u8::from(matches!(workbook.date_system, super::WorkbookDateSystem::Excel1904)))
}

fn workbook_relationships(workbook: &WorkbookIr) -> String {
    let mut relationships = workbook.worksheets.iter().enumerate().map(|(index, _)| format!("<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet{}.xml\"/>", index + 1, index + 1)).collect::<String>();
    relationships.push_str(&format!("<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>", workbook.worksheets.len() + 1));
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{relationships}</Relationships>")
}

fn attribute(xml: &str, name: &str) -> Option<String> {
    for quote in ['"', '\''] {
        let needle = format!("{name}={quote}");
        let start = xml.find(&needle)? + needle.len();
        let end = xml[start..].find(quote)? + start;
        return Some(xml[start..end].to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifacts::workbooks::deterministic_fixture;
    use crate::artifacts::workbooks::zip::read_zip;

    #[test]
    fn generated_package_contains_editable_workbook_parts_and_embedded_ir() {
        let workbook = deterministic_fixture().unwrap();
        let output = build_workbook(&workbook).unwrap();
        let entries = read_zip(&output.bytes).unwrap();
        assert!(entries.contains_key("xl/tables/table1.xml"));
        assert!(entries
            .keys()
            .any(|name| name.starts_with("xl/charts/chart")));
        let chart =
            String::from_utf8(entries.get("xl/charts/chart1.xml").unwrap().clone()).unwrap();
        assert!(chart.contains("<a:rPr lang=\"en-US\"/>"));
        assert!(
            chart.contains("<c:strCache><c:ptCount val=\"3\"/>")
                && chart.contains("<c:v>North</c:v>")
        );
        assert!(
            chart
                .contains("<c:numCache><c:formatCode>General</c:formatCode><c:ptCount val=\"3\"/>")
                && chart.contains("<c:v>1200</c:v>")
        );
        assert_eq!(extract_embedded_ir(&entries).unwrap(), output.workbook);

        let mut localized = workbook;
        localized.locale = "fr-FR".to_string();
        localized.worksheets[0].charts[0].title = "Ventes & prévisions".to_string();
        let entries = package_entries(&localized).unwrap();
        let chart = String::from_utf8(entries["xl/charts/chart1.xml"].clone()).unwrap();
        assert!(chart.contains("lang=\"fr-FR\""));
        assert!(chart.contains("Ventes &amp; prévisions"));
    }

    #[test]
    fn embedded_ir_rejects_multiple_or_malformed_roots() {
        let workbook = deterministic_fixture().unwrap();
        let mut entries = package_entries(&workbook).unwrap();
        entries.insert("customXml/item1.xml".into(), b"<?xml version=\"1.0\"?><oomuWorkbookIr sha256=\"00\"></oomuWorkbookIr><oomuWorkbookIr></oomuWorkbookIr>".to_vec());
        assert!(extract_embedded_ir(&entries)
            .unwrap_err()
            .contains("exactly one"));
        entries.insert(
            "customXml/item1.xml".into(),
            b"<?xml version=\"1.0\"?><oomuWorkbookIr sha256=\"00\">bad".to_vec(),
        );
        assert!(extract_embedded_ir(&entries)
            .unwrap_err()
            .contains("closing tag"));
    }

    #[test]
    fn build_requalifies_forged_formula_receipts_and_marks_unsupported_formulas_stale() {
        let mut forged = deterministic_fixture().unwrap();
        if let super::super::CellValue::Formula { cached_value, .. } = &mut forged.worksheets[0]
            .cells
            .iter_mut()
            .find(|cell| cell.address == "B5")
            .unwrap()
            .value
        {
            *cached_value = Some(super::super::FormulaResult::Number { value: 999_999.0 });
        }
        forged.recalculation.recalculated_at_ms = Some(-1);
        let output = build_workbook(&forged).unwrap();
        assert_eq!(
            output.workbook.worksheets[0]
                .cells
                .iter()
                .find(|cell| cell.address == "B5")
                .unwrap()
                .value,
            super::super::CellValue::Formula {
                expression: "SUM(B2:B4)".into(),
                cached_value: Some(super::super::FormulaResult::Number { value: 3_900.0 })
            }
        );
        assert_ne!(output.workbook.recalculation.recalculated_at_ms, Some(-1));
        assert!(output.verification.formulas_verified);
        #[cfg(target_os = "macos")]
        {
            assert!(output.verification.visually_verified);
            assert!(output.verification.exportable);
            assert!(output
                .verification
                .evidence
                .iter()
                .any(|check| { check.code == "exact_package_pages_rendered" && check.passed }));
        }
        #[cfg(not(target_os = "macos"))]
        {
            assert!(!output.verification.visually_verified);
            assert!(!output.verification.exportable);
        }

        let formula = output.workbook.worksheets[0]
            .cells
            .iter()
            .position(|cell| cell.address == "B5")
            .unwrap();
        let mut unsupported = output.workbook;
        unsupported.worksheets[0].cells[formula].value = super::super::CellValue::Formula {
            expression: "UNSUPPORTED(B2:B4)".into(),
            cached_value: Some(super::super::FormulaResult::Number { value: 3_900.0 }),
        };
        let output = build_workbook(&unsupported).unwrap();
        assert_eq!(
            output.workbook.recalculation.status,
            RecalculationStatus::Stale
        );
        assert!(!output.verification.exportable);
    }
}
