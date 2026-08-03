use super::*;
use crate::artifacts::workbooks::{deterministic_fixture, ProvenanceReference};

#[test]
fn bounded_instruction_infers_an_explicit_range_and_recalculates() {
    let base = deterministic_fixture().unwrap();
    let revised = revise_range(
        &base,
        &WorkbookRangeRevision {
            sheet_id: "quarterly_sales".into(),
            target_range: None,
            instruction: "set B2 to number: 1500".into(),
            replacement_cells: None,
        },
    )
    .unwrap();
    assert_eq!(revised.revision, 2);
    assert_eq!(
        revised.worksheets[0]
            .cells
            .iter()
            .find(|cell| cell.address == "B5")
            .unwrap()
            .value,
        CellValue::Formula {
            expression: "SUM(B2:B4)".into(),
            cached_value: Some(super::super::FormulaResult::Number { value: 4_200.0 })
        }
    );
}

#[test]
fn ambiguous_text_and_unbounded_instructions_return_stable_codes() {
    let mut base = deterministic_fixture().unwrap();
    base.worksheets[0].cells.push(WorkbookCell {
        address: "A6".into(),
        value: CellValue::Text {
            value: "North".into(),
        },
        format_id: None,
        comment: None,
        provenance: vec![],
    });
    let error = revise_range(
        &base,
        &WorkbookRangeRevision {
            sheet_id: "quarterly_sales".into(),
            target_range: None,
            instruction: "replace text \"North\" with \"Northeast\"".into(),
            replacement_cells: None,
        },
    )
    .unwrap_err();
    assert_eq!(
        error.code,
        WorkbookRevisionErrorCode::WorkbookRevisionTargetAmbiguous
    );
    let error = revise_range(
        &base,
        &WorkbookRangeRevision {
            sheet_id: "quarterly_sales".into(),
            target_range: None,
            instruction: "make it better".into(),
            replacement_cells: None,
        },
    )
    .unwrap_err();
    assert_eq!(
        error.code,
        WorkbookRevisionErrorCode::WorkbookRevisionTargetRequired
    );
}

#[test]
fn imported_revision_preserves_every_unrelated_part_byte_for_byte() {
    let workbook = deterministic_fixture().unwrap();
    let mut entries = super::super::ooxml::package_entries(&workbook).unwrap();
    entries.remove("customXml/item1.xml");
    for (name, fragment) in [("_rels/.rels", "<Relationship Id=\"rId4\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml\" Target=\"customXml/item1.xml\"/>"), ("[Content_Types].xml", "<Override PartName=\"/customXml/item1.xml\" ContentType=\"application/vnd.oomu.workbook-ir+xml\"/>")] {
            let xml = String::from_utf8(entries.get(name).unwrap().clone()).unwrap().replace(fragment, "");
            entries.insert(name.to_string(), xml.into_bytes());
        }
    let original = super::super::zip::write_store_zip(&entries).unwrap();
    let request = WorkbookRangeRevision {
        sheet_id: "unused".into(),
        target_range: Some("B2".into()),
        instruction: "Set the selected value".into(),
        replacement_cells: Some(vec![WorkbookCell {
            address: "B2".into(),
            value: CellValue::Number { value: 1_500.0 },
            format_id: None,
            comment: None,
            provenance: vec![],
        }]),
    };
    let revised = revise_imported_xlsx(&original, "Quarterly Sales", &request).unwrap();
    let revised_entries = super::super::zip::read_zip(&revised.bytes).unwrap();
    for (name, bytes) in &entries {
        if name != &revised.target_part && name != "xl/workbook.xml" {
            assert_eq!(
                revised_entries.get(name),
                Some(bytes),
                "unrelated part {name} changed"
            );
        }
    }
    assert!(
        String::from_utf8_lossy(revised_entries.get("xl/workbook.xml").unwrap())
            .contains("fullCalcOnLoad=\"1\"")
    );
    assert_eq!(revised.changed_parts.len(), 2);
    assert!(
        revised.changed_parts.contains(&revised.target_part)
            && revised
                .changed_parts
                .contains(&"xl/workbook.xml".to_string())
    );
    assert!(
        String::from_utf8_lossy(revised_entries.get(&revised.target_part).unwrap())
            .contains("<c r=\"B2\" s=\"2\"><v>1500</v></c>")
    );
    assert!(revised.preserved_part_digests.contains_key("xl/styles.xml"));

    let mut active = entries;
    active.insert("xl/vbaProject.bin".into(), vec![1, 2, 3]);
    let active = super::super::zip::write_store_zip(&active).unwrap();
    assert_eq!(
        revise_imported_xlsx(&active, "Quarterly Sales", &request)
            .unwrap_err()
            .code,
        WorkbookRevisionErrorCode::WorkbookRevisionUnsafePackage
    );
}

#[test]
fn imported_revision_rejects_duplicate_targets_wrong_relationship_type_and_root_escape() {
    let workbook = deterministic_fixture().unwrap();
    let mut entries = super::super::ooxml::package_entries(&workbook).unwrap();
    entries.remove("customXml/item1.xml");
    for (name, fragment) in [
        (
            "_rels/.rels",
            "<Relationship Id=\"rId4\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml\" Target=\"customXml/item1.xml\"/>",
        ),
        (
            "[Content_Types].xml",
            "<Override PartName=\"/customXml/item1.xml\" ContentType=\"application/vnd.oomu.workbook-ir+xml\"/>",
        ),
    ] {
        let xml = String::from_utf8(entries.get(name).unwrap().clone())
            .unwrap()
            .replace(fragment, "");
        entries.insert(name.to_string(), xml.into_bytes());
    }
    let replacement = WorkbookCell {
        address: "B2".into(),
        value: CellValue::Number { value: 1_500.0 },
        format_id: None,
        comment: None,
        provenance: vec![],
    };
    let duplicate_request = WorkbookRangeRevision {
        sheet_id: "unused".into(),
        target_range: Some("B2".into()),
        instruction: "Set the selected value".into(),
        replacement_cells: Some(vec![replacement.clone(), replacement.clone()]),
    };
    let original = super::super::zip::write_store_zip(&entries).unwrap();
    assert_eq!(
        revise_imported_xlsx(&original, "Quarterly Sales", &duplicate_request)
            .unwrap_err()
            .code,
        WorkbookRevisionErrorCode::WorkbookRevisionTargetMismatch
    );

    let rels = "xl/_rels/workbook.xml.rels";
    let mutated = String::from_utf8(entries.get(rels).unwrap().clone())
        .unwrap()
        .replacen("/worksheet\"", "/styles\"", 1);
    entries.insert(rels.to_string(), mutated.into_bytes());
    let safe_request = WorkbookRangeRevision {
        replacement_cells: Some(vec![replacement]),
        ..duplicate_request
    };
    let wrong_type = super::super::zip::write_store_zip(&entries).unwrap();
    assert_eq!(
        revise_imported_xlsx(&wrong_type, "Quarterly Sales", &safe_request)
            .unwrap_err()
            .code,
        WorkbookRevisionErrorCode::WorkbookRevisionUnsafePackage
    );
    assert!(resolve_xl_target("../../outside.xml").is_err());
}

#[test]
fn obvious_selected_cell_language_is_inferred_conservatively() {
    let base = deterministic_fixture().unwrap();
    let amount = revise_range(
        &base,
        &WorkbookRangeRevision {
            sheet_id: "quarterly_sales".into(),
            target_range: Some("B2".into()),
            instruction: "Set to 500".into(),
            replacement_cells: None,
        },
    )
    .unwrap();
    assert_eq!(
        amount.worksheets[0]
            .cells
            .iter()
            .find(|cell| cell.address == "B2")
            .unwrap()
            .value,
        CellValue::Number { value: 500.0 }
    );
    let status = revise_range(
        &base,
        &WorkbookRangeRevision {
            sheet_id: "quarterly_sales".into(),
            target_range: Some("D4".into()),
            instruction: "Change to Approved".into(),
            replacement_cells: None,
        },
    )
    .unwrap();
    assert_eq!(
        status.worksheets[0]
            .cells
            .iter()
            .find(|cell| cell.address == "D4")
            .unwrap()
            .value,
        CellValue::Text {
            value: "Approved".into()
        }
    );
    let cleared = revise_range(
        &base,
        &WorkbookRangeRevision {
            sheet_id: "quarterly_sales".into(),
            target_range: Some("D4".into()),
            instruction: "Clear these cells".into(),
            replacement_cells: None,
        },
    )
    .unwrap();
    assert_eq!(
        cleared.worksheets[0]
            .cells
            .iter()
            .find(|cell| cell.address == "D4")
            .unwrap()
            .value,
        CellValue::Blank
    );
    let replaced = revise_range(
        &base,
        &WorkbookRangeRevision {
            sheet_id: "quarterly_sales".into(),
            target_range: None,
            instruction: "Replace “North” with “Northeast”".into(),
            replacement_cells: None,
        },
    )
    .unwrap();
    assert_eq!(
        replaced.worksheets[0]
            .cells
            .iter()
            .find(|cell| cell.address == "A2")
            .unwrap()
            .value,
        CellValue::Text {
            value: "Northeast".into()
        }
    );
    let formula = revise_range(
        &base,
        &WorkbookRangeRevision {
            sheet_id: "quarterly_sales".into(),
            target_range: Some("B2".into()),
            instruction: "Set to =SUM(B2:B4)".into(),
            replacement_cells: None,
        },
    )
    .unwrap_err();
    assert_eq!(
        formula.code,
        WorkbookRevisionErrorCode::WorkbookRevisionInstructionUnsupported
    );
}

#[test]
fn value_mutations_clear_changed_and_recalculated_lineage_only() {
    let mut base = deterministic_fixture().unwrap();
    let provenance = ProvenanceReference {
        source_ref: "connector.tool.completed".into(),
        evidence_ref: "task-event:taskrun_00000000-0000-4000-8000-000000000000:1".into(),
        note: None,
    };
    for address in ["A2", "A3", "B2", "B5"] {
        base.worksheets[0]
            .cells
            .iter_mut()
            .find(|cell| cell.address == address)
            .unwrap()
            .provenance = vec![provenance.clone()];
    }
    let replaced = revise_range(
        &base,
        &WorkbookRangeRevision {
            sheet_id: "quarterly_sales".into(),
            target_range: Some("A2".into()),
            instruction: "Replace “North” with “Northeast”".into(),
            replacement_cells: None,
        },
    )
    .unwrap();
    let cell = |address: &str| {
        replaced.worksheets[0]
            .cells
            .iter()
            .find(|cell| cell.address == address)
            .unwrap()
    };
    assert!(cell("A2").provenance.is_empty());
    assert_eq!(cell("A3").provenance, vec![provenance.clone()]);
    assert!(cell("B5").provenance.is_empty());

    let carried = revise_range(
        &base,
        &WorkbookRangeRevision {
            sheet_id: "quarterly_sales".into(),
            target_range: Some("B2".into()),
            instruction: "Set exact value".into(),
            replacement_cells: Some(vec![WorkbookCell {
                address: "B2".into(),
                value: CellValue::Number { value: 500.0 },
                format_id: None,
                comment: None,
                provenance: vec![provenance.clone()],
            }]),
        },
    )
    .unwrap();
    assert!(carried.worksheets[0]
        .cells
        .iter()
        .find(|cell| cell.address == "B2")
        .unwrap()
        .provenance
        .is_empty());
    assert!(carried.worksheets[0]
        .cells
        .iter()
        .find(|cell| cell.address == "B5")
        .unwrap()
        .provenance
        .is_empty());
}
