use super::{address::parse_cell_address, CellValue, FormulaResult, WorkbookIr, Worksheet};
use std::collections::HashMap;

pub(crate) const PAGE_SETUP_PROPERTIES_XML: &str =
    "<sheetPr><pageSetUpPr fitToPage=\"1\" autoPageBreaks=\"0\"/></sheetPr>";
const LETTER_PAGE_SETTINGS_XML: &str = "<printOptions horizontalCentered=\"1\" gridLines=\"0\" headings=\"0\"/><pageMargins left=\"0.25\" right=\"0.25\" top=\"0.50\" bottom=\"0.50\" header=\"0.20\" footer=\"0.20\"/><pageSetup paperSize=\"1\" orientation=\"landscape\" fitToWidth=\"1\" fitToHeight=\"0\" firstPageNumber=\"1\" useFirstPageNumber=\"1\"/>";
const TABLOID_PAGE_SETTINGS_XML: &str = "<printOptions horizontalCentered=\"1\" gridLines=\"0\" headings=\"0\"/><pageMargins left=\"0.25\" right=\"0.25\" top=\"0.50\" bottom=\"0.50\" header=\"0.20\" footer=\"0.20\"/><pageSetup paperSize=\"3\" orientation=\"landscape\" fitToWidth=\"1\" fitToHeight=\"0\" firstPageNumber=\"1\" useFirstPageNumber=\"1\"/>";
const WIDE_SHEET_WIDTH_THRESHOLD: f64 = 180.0;

pub(crate) struct SheetPrintLayout<'a> {
    formats: HashMap<&'a str, &'a super::CellFormat>,
    width_by_column: HashMap<u32, f64>,
    total_configured_width: f64,
}

impl<'a> SheetPrintLayout<'a> {
    pub(crate) fn new(workbook: &'a WorkbookIr, sheet: &Worksheet) -> Self {
        let formats = workbook
            .formats
            .iter()
            .map(|format| (format.format_id.as_str(), format))
            .collect();
        let width_by_column: HashMap<_, _> = sheet
            .column_widths
            .iter()
            .filter_map(|width| {
                parse_cell_address(&format!("{}1", width.column))
                    .ok()
                    .map(|address| (address.column, width.width))
            })
            .collect();
        let total_configured_width = width_by_column.values().sum();
        Self {
            formats,
            width_by_column,
            total_configured_width,
        }
    }

    pub(crate) fn page_settings_xml(&self) -> &'static str {
        if self.total_configured_width > WIDE_SHEET_WIDTH_THRESHOLD {
            TABLOID_PAGE_SETTINGS_XML
        } else {
            LETTER_PAGE_SETTINGS_XML
        }
    }

    pub(crate) fn wrapped_row_height(&self, cells: &[(u32, &super::WorkbookCell)]) -> Option<f64> {
        cells
            .iter()
            .filter_map(|(column, cell)| self.wrapped_cell_height(*column, cell))
            .reduce(f64::max)
    }

    fn wrapped_cell_height(&self, column: u32, cell: &super::WorkbookCell) -> Option<f64> {
        let format = cell
            .format_id
            .as_deref()
            .and_then(|format_id| self.formats.get(format_id).copied());
        let text = printable_text(&cell.value)?;
        let wraps =
            format.map(|format| format.wrap_text).unwrap_or(false) || text.contains(['\n', '\r']);
        if !wraps {
            return None;
        }
        let width = self.width_by_column.get(&column).copied().unwrap_or(8.43);
        let characters_per_line = (width * 1.05).floor().max(4.0) as usize;
        let lines = text
            .lines()
            .map(|line| line.chars().count().max(1).div_ceil(characters_per_line))
            .sum::<usize>()
            .max(1);
        let font_size = format
            .and_then(|format| format.font.size_pt)
            .unwrap_or(11.0) as f64;
        (lines > 1).then(|| (lines as f64 * (font_size * 1.25 + 2.0)).clamp(15.0, 180.0))
    }
}

fn printable_text(value: &CellValue) -> Option<&str> {
    match value {
        CellValue::Text { value } => Some(value),
        CellValue::Formula {
            cached_value: Some(FormulaResult::Text { value }),
            ..
        } => Some(value),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifacts::workbooks::deterministic_fixture;

    #[test]
    fn print_contract_is_landscape_and_one_page_wide() {
        let workbook = deterministic_fixture().unwrap();
        let settings =
            SheetPrintLayout::new(&workbook, &workbook.worksheets[0]).page_settings_xml();
        assert!(PAGE_SETUP_PROPERTIES_XML.contains("fitToPage=\"1\""));
        assert!(settings.contains("paperSize=\"1\""));
        assert!(settings.contains("orientation=\"landscape\""));
        assert!(settings.contains("fitToWidth=\"1\""));
        assert!(settings.contains("fitToHeight=\"0\""));
        assert!(settings.find("<pageMargins").unwrap() < settings.find("<pageSetup").unwrap());
    }

    #[test]
    fn wide_sheets_use_standard_tabloid_landscape_paper() {
        let mut workbook = deterministic_fixture().unwrap();
        workbook.worksheets[0].column_widths[0].width = 200.0;
        let settings =
            SheetPrintLayout::new(&workbook, &workbook.worksheets[0]).page_settings_xml();

        assert!(settings.contains("paperSize=\"3\""));
        assert!(settings.contains("orientation=\"landscape\""));
    }

    #[test]
    fn wrapped_content_receives_a_content_aware_row_height() {
        let mut workbook = deterministic_fixture().unwrap();
        let note = workbook.worksheets[1]
            .cells
            .iter_mut()
            .find(|cell| cell.address == "B2")
            .unwrap();
        note.value = CellValue::Text {
            value: "This evidence note deliberately wraps across several visible lines so the printed workbook preserves every word.".to_string(),
        };
        let sheet = &workbook.worksheets[1];
        let cells = sheet
            .cells
            .iter()
            .filter_map(|cell| {
                let address = parse_cell_address(&cell.address).unwrap();
                (address.row == 2).then_some((address.column, cell))
            })
            .collect::<Vec<_>>();

        assert_eq!(
            SheetPrintLayout::new(&workbook, sheet).wrapped_row_height(&cells),
            Some(58.0)
        );
    }
}
