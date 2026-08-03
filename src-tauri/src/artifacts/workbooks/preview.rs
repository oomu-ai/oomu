use super::{
    address::parse_cell_address, CellValue, FormulaResult, WorkbookIr, WorkbookLocation,
    WorkbookPreviewEvidence, WorkbookPreviewImage, WorkbookWarning, WorkbookWarningCode, Worksheet,
};
use crate::foundation::digest::sha256_hex;
use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
use std::{collections::HashMap, io::Cursor};

const CANVAS_WIDTH: u32 = 1_200;
const CANVAS_HEIGHT: u32 = 800;
const HEADER_HEIGHT: u32 = 48;
const ROW_HEIGHT: u32 = 24;
const ROW_LABEL_WIDTH: u32 = 42;

pub(crate) fn render_previews(
    workbook: &WorkbookIr,
) -> Result<(Vec<WorkbookPreviewImage>, Vec<WorkbookWarning>), String> {
    render_previews_with_native_result(
        workbook,
        super::native_preview::render_native_previews(workbook),
    )
}

pub(super) fn render_previews_with_native_result(
    workbook: &WorkbookIr,
    native: Result<(Vec<WorkbookPreviewImage>, Vec<WorkbookWarning>), String>,
) -> Result<(Vec<WorkbookPreviewImage>, Vec<WorkbookWarning>), String> {
    if let Ok(rendered) = native {
        return Ok(rendered);
    }
    let mut images = Vec::new();
    let mut warnings = vec![WorkbookWarning {
        code: WorkbookWarningCode::PreviewUnavailable,
        location: WorkbookLocation::default(),
        technical_detail: "The qualified sheet renderer was unavailable. The fallback image is for orientation only and cannot authorize export.".to_string(),
    }];
    for sheet in &workbook.worksheets {
        let (image, mut sheet_warnings) = render_sheet(workbook, sheet)?;
        warnings.append(&mut sheet_warnings);
        images.push(image);
    }
    Ok((images, warnings))
}

fn render_sheet(
    workbook: &WorkbookIr,
    sheet: &Worksheet,
) -> Result<(WorkbookPreviewImage, Vec<WorkbookWarning>), String> {
    let mut canvas = RgbaImage::from_pixel(CANVAS_WIDTH, CANVAS_HEIGHT, Rgba([250, 250, 250, 255]));
    let mut warnings = Vec::new();
    let first_unsupported = sheet
        .cells
        .iter()
        .find(|cell| !display_value(&cell.value).is_ascii())
        .map(|cell| cell.address.clone());
    if !workbook.title.is_ascii()
        || !sheet.name.is_ascii()
        || first_unsupported.is_some()
        || sheet.charts.iter().any(|chart| {
            !chart.title.is_ascii() || chart.series.iter().any(|series| !series.name.is_ascii())
        })
    {
        warnings.push(warning(WorkbookWarningCode::PreviewUnsupportedCharacters, sheet, first_unsupported, None, "The bundled fallback preview cannot prove non-ASCII glyph rendering; qualified office-renderer review is required."));
    }
    fill_rect(
        &mut canvas,
        0,
        0,
        CANVAS_WIDTH,
        HEADER_HEIGHT,
        Rgba([31, 41, 55, 255]),
    );
    draw_text(
        &mut canvas,
        18,
        16,
        &format!("{} / {}", workbook.title, sheet.name),
        Rgba([255, 255, 255, 255]),
        2,
    );
    let width_map = sheet
        .column_widths
        .iter()
        .filter_map(|value| {
            parse_cell_address(&format!("{}1", value.column))
                .ok()
                .map(|address| (address.column, value.width))
        })
        .collect::<HashMap<_, _>>();
    let mut columns = Vec::new();
    let mut x = ROW_LABEL_WIDTH;
    for column in 1..=sheet.bounds.column_count.min(20) {
        let width = width_map.get(&column).copied().unwrap_or(12.0);
        let pixels = (width * 7.0 + 10.0).clamp(42.0, 240.0) as u32;
        if x + pixels > CANVAS_WIDTH {
            break;
        }
        columns.push((column, x, pixels));
        x += pixels;
    }
    let max_rows =
        ((CANVAS_HEIGHT - HEADER_HEIGHT - ROW_HEIGHT) / ROW_HEIGHT).min(sheet.bounds.row_count);
    if max_rows < sheet.bounds.row_count || columns.len() < sheet.bounds.column_count as usize {
        warnings.push(warning(
            WorkbookWarningCode::PreviewTruncated,
            sheet,
            None,
            None,
            "Preview shows the leading visible region; workbook data remains complete.",
        ));
    }
    fill_rect(
        &mut canvas,
        0,
        HEADER_HEIGHT,
        CANVAS_WIDTH,
        ROW_HEIGHT,
        Rgba([229, 231, 235, 255]),
    );
    for (column, start, width) in &columns {
        stroke_rect(
            &mut canvas,
            *start,
            HEADER_HEIGHT,
            *width,
            ROW_HEIGHT,
            Rgba([156, 163, 175, 255]),
        );
        draw_text(
            &mut canvas,
            *start + 5,
            HEADER_HEIGHT + 8,
            &super::address::column_name(*column),
            Rgba([31, 41, 55, 255]),
            1,
        );
    }
    let cells = sheet
        .cells
        .iter()
        .filter_map(|cell| {
            parse_cell_address(&cell.address)
                .ok()
                .map(|address| ((address.row, address.column), cell))
        })
        .collect::<HashMap<_, _>>();
    let formats = workbook
        .formats
        .iter()
        .map(|format| (format.format_id.as_str(), format))
        .collect::<HashMap<_, _>>();
    for row in 1..=max_rows {
        let y = HEADER_HEIGHT + ROW_HEIGHT * row;
        fill_rect(
            &mut canvas,
            0,
            y,
            ROW_LABEL_WIDTH,
            ROW_HEIGHT,
            Rgba([243, 244, 246, 255]),
        );
        draw_text(
            &mut canvas,
            6,
            y + 8,
            &row.to_string(),
            Rgba([75, 85, 99, 255]),
            1,
        );
        for (column, start, width) in &columns {
            let background = cells
                .get(&(row, *column))
                .and_then(|cell| cell.format_id.as_ref())
                .and_then(|id| formats.get(id.as_str()))
                .and_then(|format| format.fill_color.as_deref())
                .and_then(parse_rgb)
                .unwrap_or(Rgba([255, 255, 255, 255]));
            fill_rect(&mut canvas, *start, y, *width, ROW_HEIGHT, background);
            stroke_rect(
                &mut canvas,
                *start,
                y,
                *width,
                ROW_HEIGHT,
                Rgba([209, 213, 219, 255]),
            );
            if let Some(cell) = cells.get(&(row, *column)) {
                let display = display_value(&cell.value);
                let wrap = cell
                    .format_id
                    .as_ref()
                    .and_then(|id| formats.get(id.as_str()))
                    .map(|format| format.wrap_text)
                    .unwrap_or(false);
                if display.chars().count() as u32 * 6 > width.saturating_sub(8) && !wrap {
                    warnings.push(warning(
                        WorkbookWarningCode::ColumnContentClipped,
                        sheet,
                        Some(cell.address.clone()),
                        None,
                        "Cell text is wider than its configured preview column.",
                    ));
                }
                draw_text(
                    &mut canvas,
                    *start + 4,
                    y + 8,
                    &truncate_for_width(&display, width.saturating_sub(8)),
                    Rgba([17, 24, 39, 255]),
                    1,
                );
            }
        }
    }
    for chart in &sheet.charts {
        let x = ROW_LABEL_WIDTH + chart.anchor.from_column.saturating_mul(70);
        let y = HEADER_HEIGHT + ROW_HEIGHT + chart.anchor.from_row.saturating_mul(ROW_HEIGHT);
        let width = chart
            .anchor
            .to_column
            .saturating_sub(chart.anchor.from_column)
            .saturating_mul(70)
            .min(CANVAS_WIDTH.saturating_sub(x));
        let height = chart
            .anchor
            .to_row
            .saturating_sub(chart.anchor.from_row)
            .saturating_mul(ROW_HEIGHT)
            .min(CANVAS_HEIGHT.saturating_sub(y));
        if width > 20 && height > 20 {
            fill_rect(&mut canvas, x, y, width, height, Rgba([255, 255, 255, 245]));
            stroke_rect(&mut canvas, x, y, width, height, Rgba([75, 85, 99, 255]));
            draw_text(
                &mut canvas,
                x + 10,
                y + 10,
                &chart.title,
                Rgba([31, 41, 55, 255]),
                1,
            );
            draw_chart_marks(
                &mut canvas,
                x + 12,
                y + 28,
                width.saturating_sub(24),
                height.saturating_sub(40),
                chart.series.len(),
            );
        }
    }
    let mut encoded = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(canvas)
        .write_to(&mut encoded, ImageFormat::Png)
        .map_err(|error| error.to_string())?;
    let bytes = encoded.into_inner();
    let evidence = WorkbookPreviewEvidence {
        sheet_id: sheet.sheet_id.clone(),
        mime_type: "image/png".to_string(),
        width: CANVAS_WIDTH,
        height: CANVAS_HEIGHT,
        sha256: sha256_hex(&bytes),
    };
    Ok((WorkbookPreviewImage { evidence, bytes }, warnings))
}

fn display_value(value: &CellValue) -> String {
    match value {
        CellValue::Blank => String::new(),
        CellValue::Text { value } => value.clone(),
        CellValue::Number { value } => format!("{value}"),
        CellValue::Boolean { value } => if *value { "TRUE" } else { "FALSE" }.to_string(),
        CellValue::Date { iso } => iso.clone(),
        CellValue::Formula {
            expression,
            cached_value,
        } => match cached_value {
            Some(FormulaResult::Number { value }) => value.to_string(),
            Some(FormulaResult::Text { value }) => value.clone(),
            Some(FormulaResult::Boolean { value }) => {
                if *value { "TRUE" } else { "FALSE" }.to_string()
            }
            Some(FormulaResult::Error { code }) => code.clone(),
            None => format!("={}", expression.strip_prefix('=').unwrap_or(expression)),
        },
    }
}

fn warning(
    code: WorkbookWarningCode,
    sheet: &Worksheet,
    range: Option<String>,
    chart_id: Option<String>,
    detail: &str,
) -> WorkbookWarning {
    WorkbookWarning {
        code,
        location: WorkbookLocation {
            sheet_id: Some(sheet.sheet_id.clone()),
            range,
            chart_id,
        },
        technical_detail: detail.to_string(),
    }
}

fn truncate_for_width(value: &str, width: u32) -> String {
    let limit = (width / 6).max(1) as usize;
    if value.chars().count() <= limit {
        return value.to_string();
    }
    value
        .chars()
        .take(limit.saturating_sub(1))
        .collect::<String>()
        + "…"
}

fn parse_rgb(value: &str) -> Option<Rgba<u8>> {
    if value.len() != 6 {
        return None;
    }
    Some(Rgba([
        u8::from_str_radix(&value[0..2], 16).ok()?,
        u8::from_str_radix(&value[2..4], 16).ok()?,
        u8::from_str_radix(&value[4..6], 16).ok()?,
        255,
    ]))
}

fn fill_rect(image: &mut RgbaImage, x: u32, y: u32, width: u32, height: u32, color: Rgba<u8>) {
    for row in y..y.saturating_add(height).min(image.height()) {
        for column in x..x.saturating_add(width).min(image.width()) {
            image.put_pixel(column, row, color);
        }
    }
}

fn stroke_rect(image: &mut RgbaImage, x: u32, y: u32, width: u32, height: u32, color: Rgba<u8>) {
    if width == 0 || height == 0 {
        return;
    }
    for column in x..x.saturating_add(width).min(image.width()) {
        if y < image.height() {
            image.put_pixel(column, y, color);
        }
        let bottom = y.saturating_add(height - 1);
        if bottom < image.height() {
            image.put_pixel(column, bottom, color);
        }
    }
    for row in y..y.saturating_add(height).min(image.height()) {
        if x < image.width() {
            image.put_pixel(x, row, color);
        }
        let right = x.saturating_add(width - 1);
        if right < image.width() {
            image.put_pixel(right, row, color);
        }
    }
}

fn draw_chart_marks(image: &mut RgbaImage, x: u32, y: u32, width: u32, height: u32, series: usize) {
    if width < 10 || height < 10 {
        return;
    }
    let colors = [
        Rgba([37, 99, 235, 255]),
        Rgba([16, 185, 129, 255]),
        Rgba([245, 158, 11, 255]),
    ];
    for index in 0..series.max(1).min(3) {
        let bar_width = width / 8;
        let bar_height = height.saturating_mul((index + 2) as u32) / 4;
        fill_rect(
            image,
            x + (index as u32 * (bar_width + 12)),
            y + height.saturating_sub(bar_height),
            bar_width,
            bar_height,
            colors[index],
        );
    }
}

fn draw_text(image: &mut RgbaImage, x: u32, y: u32, text: &str, color: Rgba<u8>, scale: u32) {
    let mut cursor = x;
    for character in text.chars() {
        if cursor + 6 * scale >= image.width() {
            break;
        }
        let glyph = glyph(character);
        for (row, bits) in glyph.iter().enumerate() {
            for column in 0..5 {
                if bits & (1 << (4 - column)) != 0 {
                    fill_rect(
                        image,
                        cursor + column * scale,
                        y + row as u32 * scale,
                        scale,
                        scale,
                        color,
                    );
                }
            }
        }
        cursor += 6 * scale;
    }
}

fn glyph(character: char) -> [u8; 7] {
    match character.to_ascii_uppercase() {
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'C' => [14, 17, 16, 16, 16, 17, 14],
        'D' => [30, 17, 17, 17, 17, 17, 30],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'G' => [14, 17, 16, 23, 17, 17, 15],
        'H' => [17, 17, 17, 31, 17, 17, 17],
        'I' => [31, 4, 4, 4, 4, 4, 31],
        'J' => [7, 2, 2, 2, 18, 18, 12],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'N' => [17, 25, 21, 19, 17, 17, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'Q' => [14, 17, 17, 17, 21, 18, 13],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'W' => [17, 17, 17, 21, 21, 21, 10],
        'X' => [17, 17, 10, 4, 10, 17, 17],
        'Y' => [17, 17, 10, 4, 4, 4, 4],
        'Z' => [31, 1, 2, 4, 8, 16, 31],
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [30, 1, 1, 14, 1, 1, 30],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 16, 30, 1, 1, 30],
        '6' => [14, 16, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 1, 14],
        '-' => [0, 0, 0, 31, 0, 0, 0],
        '_' => [0, 0, 0, 0, 0, 0, 31],
        '.' => [0, 0, 0, 0, 0, 12, 12],
        '/' => [1, 2, 2, 4, 8, 8, 16],
        ':' => [0, 12, 12, 0, 12, 12, 0],
        ' ' => [0; 7],
        _ => [31, 17, 1, 2, 4, 0, 4],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifacts::workbooks::deterministic_fixture;

    #[test]
    fn renders_a_deterministic_png_for_each_sheet() {
        let workbook = deterministic_fixture().unwrap();
        let (images, _) = render_previews(&workbook).unwrap();
        assert_eq!(images.len(), workbook.worksheets.len());
        assert!(images
            .iter()
            .all(|image| image.bytes.starts_with(b"\x89PNG\r\n\x1a\n")));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn non_ascii_content_is_authorized_only_by_exact_package_qualification() {
        let mut workbook = deterministic_fixture().unwrap();
        workbook.title = "四半期売上".into();
        let output = super::super::build_workbook(&workbook).unwrap();
        assert!(output.verification.visually_verified);
        assert!(output.verification.exportable);
        assert!(!output
            .verification
            .warnings
            .iter()
            .any(|warning| warning.code == WorkbookWarningCode::PreviewUnavailable));
        assert!(output
            .verification
            .evidence
            .iter()
            .any(|check| { check.code == "exact_package_pages_rendered" && check.passed }));
    }

    #[test]
    fn native_missing_crash_or_timeout_marks_fallback_unqualified() {
        let workbook = deterministic_fixture().unwrap();
        for failure in ["renderer missing", "renderer crashed", "renderer timed out"] {
            let (images, warnings) =
                render_previews_with_native_result(&workbook, Err(failure.to_string())).unwrap();
            assert_eq!(images.len(), workbook.worksheets.len());
            assert!(warnings
                .iter()
                .any(|warning| warning.code == WorkbookWarningCode::PreviewUnavailable));
        }
    }
}
