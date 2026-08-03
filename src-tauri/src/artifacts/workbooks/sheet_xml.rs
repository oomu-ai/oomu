use super::{
    address::{
        a1, parse_cell_address, parse_local_range, quote_sheet_name, split_qualified_range,
        CellAddress,
    },
    sheet_print_xml::{SheetPrintLayout, PAGE_SETUP_PROPERTIES_XML},
    style_xml::{xml_attr, xml_text, StyleCatalog},
    CellValue, ChartKind, FormulaResult, ValidationRule, WorkbookDateSystem, WorkbookIr, Worksheet,
};
use chrono::{DateTime, NaiveDate, Timelike};
use std::collections::{BTreeMap, HashMap};

pub(crate) type WorkbookCellIndex<'a> = HashMap<(String, u32, u32), &'a CellValue>;

pub(crate) fn workbook_cell_index(workbook: &WorkbookIr) -> Result<WorkbookCellIndex<'_>, String> {
    let mut index = HashMap::new();
    for sheet in &workbook.worksheets {
        let sheet_name = sheet.name.to_lowercase();
        for cell in &sheet.cells {
            let address = parse_cell_address(&cell.address)?;
            index.insert(
                (sheet_name.clone(), address.row, address.column),
                &cell.value,
            );
        }
    }
    Ok(index)
}

pub(crate) struct SheetParts {
    pub sheet_xml: Vec<u8>,
    pub relationships: Option<Vec<u8>>,
    pub extra_parts: BTreeMap<String, Vec<u8>>,
    pub overrides: Vec<(String, String)>,
}

pub(crate) fn build_sheet_parts(
    workbook: &WorkbookIr,
    sheet: &Worksheet,
    sheet_index: usize,
    styles: &StyleCatalog,
    next_table_id: &mut u32,
    next_chart_id: &mut u32,
    cell_index: &WorkbookCellIndex<'_>,
) -> Result<SheetParts, String> {
    let mut relationships = Vec::new();
    let mut extra_parts = BTreeMap::new();
    let mut overrides = Vec::new();
    let mut next_relationship = 1_u32;
    let mut table_relationships = Vec::new();
    for table in &sheet.tables {
        let table_id = *next_table_id;
        *next_table_id += 1;
        let relationship_id = next_relationship;
        next_relationship += 1;
        relationships.push(format!("<Relationship Id=\"rId{relationship_id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/table\" Target=\"../tables/table{table_id}.xml\"/>"));
        table_relationships.push(relationship_id);
        let path = format!("xl/tables/table{table_id}.xml");
        extra_parts.insert(path.clone(), table_xml(table, table_id)?.into_bytes());
        overrides.push((
            format!("/{path}"),
            "application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml".to_string(),
        ));
    }
    let drawing_relationship = if !sheet.charts.is_empty() {
        let relationship_id = next_relationship;
        next_relationship += 1;
        relationships.push(format!("<Relationship Id=\"rId{relationship_id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing\" Target=\"../drawings/drawing{}.xml\"/>", sheet_index + 1));
        let (drawing, drawing_rels, charts) =
            drawing_parts(workbook, sheet, sheet_index, next_chart_id, cell_index)?;
        let drawing_path = format!("xl/drawings/drawing{}.xml", sheet_index + 1);
        extra_parts.insert(drawing_path.clone(), drawing.into_bytes());
        extra_parts.insert(
            format!("xl/drawings/_rels/drawing{}.xml.rels", sheet_index + 1),
            drawing_rels.into_bytes(),
        );
        overrides.push((
            format!("/{drawing_path}"),
            "application/vnd.openxmlformats-officedocument.drawing+xml".to_string(),
        ));
        for (path, xml) in charts {
            overrides.push((
                format!("/{path}"),
                "application/vnd.openxmlformats-officedocument.drawingml.chart+xml".to_string(),
            ));
            extra_parts.insert(path, xml.into_bytes());
        }
        Some(relationship_id)
    } else {
        None
    };
    let comments = sheet
        .cells
        .iter()
        .filter_map(|cell| cell.comment.as_ref().map(|comment| (cell, comment)))
        .collect::<Vec<_>>();
    let comment_relationships = if comments.is_empty() {
        None
    } else {
        let comment_id = next_relationship;
        next_relationship += 1;
        let vml_id = next_relationship;
        relationships.push(format!("<Relationship Id=\"rId{comment_id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments\" Target=\"../comments{}.xml\"/>", sheet_index + 1));
        relationships.push(format!("<Relationship Id=\"rId{vml_id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/vmlDrawing\" Target=\"../drawings/vmlDrawing{}.vml\"/>", sheet_index + 1));
        let comment_path = format!("xl/comments{}.xml", sheet_index + 1);
        extra_parts.insert(comment_path.clone(), comments_xml(&comments).into_bytes());
        extra_parts.insert(
            format!("xl/drawings/vmlDrawing{}.vml", sheet_index + 1),
            comments_vml(&comments)?.into_bytes(),
        );
        overrides.push((
            format!("/{comment_path}"),
            "application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml".to_string(),
        ));
        Some((comment_id, vml_id))
    };
    let sheet_xml = worksheet_xml(
        workbook,
        sheet,
        styles,
        &table_relationships,
        drawing_relationship,
        comment_relationships,
    )?;
    let rels = if relationships.is_empty() {
        None
    } else {
        Some(format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{}</Relationships>", relationships.join("")).into_bytes())
    };
    Ok(SheetParts {
        sheet_xml: sheet_xml.into_bytes(),
        relationships: rels,
        extra_parts,
        overrides,
    })
}

fn worksheet_xml(
    workbook: &WorkbookIr,
    sheet: &Worksheet,
    styles: &StyleCatalog,
    table_relationships: &[u32],
    drawing_relationship: Option<u32>,
    comment_relationships: Option<(u32, u32)>,
) -> Result<String, String> {
    let max_cell = sheet
        .cells
        .iter()
        .map(|cell| parse_cell_address(&cell.address))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .fold(CellAddress { row: 1, column: 1 }, |maximum, cell| {
            CellAddress {
                row: maximum.row.max(cell.row),
                column: maximum.column.max(cell.column),
            }
        });
    let mut xml = format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">{PAGE_SETUP_PROPERTIES_XML}<dimension ref=\"A1:{}\"/><sheetViews><sheetView workbookViewId=\"0\"/></sheetViews><sheetFormatPr defaultRowHeight=\"15\"/>", a1(max_cell));
    let print_layout = SheetPrintLayout::new(workbook, sheet);
    if !sheet.column_widths.is_empty() {
        xml.push_str("<cols>");
        for width in &sheet.column_widths {
            let column = parse_cell_address(&format!("{}1", width.column))?.column;
            xml.push_str(&format!(
                "<col min=\"{column}\" max=\"{column}\" width=\"{}\" customWidth=\"1\"/>",
                width.width
            ));
        }
        xml.push_str("</cols>");
    }
    let mut rows: BTreeMap<u32, Vec<_>> = BTreeMap::new();
    for cell in &sheet.cells {
        let address = parse_cell_address(&cell.address)?;
        rows.entry(address.row)
            .or_default()
            .push((address.column, cell));
    }
    xml.push_str("<sheetData>");
    for (row, mut cells) in rows {
        cells.sort_by_key(|(column, _)| *column);
        let height = print_layout.wrapped_row_height(&cells);
        let height_attribute = height
            .map(|height| format!(" ht=\"{height:.2}\" customHeight=\"1\""))
            .unwrap_or_default();
        xml.push_str(&format!("<row r=\"{row}\"{height_attribute}>"));
        for (_, cell) in cells {
            xml.push_str(&cell_xml(workbook, cell, styles)?);
        }
        xml.push_str("</row>");
    }
    xml.push_str("</sheetData>");
    if !sheet.merged_ranges.is_empty() {
        xml.push_str(&format!(
            "<mergeCells count=\"{}\">",
            sheet.merged_ranges.len()
        ));
        for range in &sheet.merged_ranges {
            xml.push_str(&format!("<mergeCell ref=\"{}\"/>", xml_attr(range)));
        }
        xml.push_str("</mergeCells>");
    }
    if !sheet.validations.is_empty() {
        xml.push_str(&format!(
            "<dataValidations count=\"{}\">",
            sheet.validations.len()
        ));
        for validation in &sheet.validations {
            xml.push_str(&validation_xml(validation)?);
        }
        xml.push_str("</dataValidations>");
    }
    xml.push_str(print_layout.page_settings_xml());
    if let Some(relationship) = drawing_relationship {
        xml.push_str(&format!("<drawing r:id=\"rId{relationship}\"/>"));
    }
    if let Some((_, vml_relationship)) = comment_relationships {
        xml.push_str(&format!("<legacyDrawing r:id=\"rId{vml_relationship}\"/>"));
    }
    if !table_relationships.is_empty() {
        xml.push_str(&format!(
            "<tableParts count=\"{}\">",
            table_relationships.len()
        ));
        for relationship in table_relationships {
            xml.push_str(&format!("<tablePart r:id=\"rId{relationship}\"/>"));
        }
        xml.push_str("</tableParts>");
    }
    xml.push_str("</worksheet>");
    Ok(xml)
}

fn cell_xml(
    workbook: &WorkbookIr,
    cell: &super::WorkbookCell,
    styles: &StyleCatalog,
) -> Result<String, String> {
    let style = cell
        .format_id
        .as_ref()
        .and_then(|value| styles.indexes.get(value))
        .copied();
    let mut attributes = format!(" r=\"{}\"", xml_attr(&cell.address));
    let body = match &cell.value {
        CellValue::Blank => String::new(),
        CellValue::Text { value } => {
            attributes.push_str(" t=\"inlineStr\"");
            format!("<is><t xml:space=\"preserve\">{}</t></is>", xml_text(value))
        }
        CellValue::Number { value } => format!("<v>{value}</v>"),
        CellValue::Boolean { value } => {
            attributes.push_str(" t=\"b\"");
            format!("<v>{}</v>", u8::from(*value))
        }
        CellValue::Date { iso } => {
            if style.is_none() {
                attributes.push_str(&format!(" s=\"{}\"", styles.date_style_index));
            }
            format!("<v>{}</v>", excel_date_serial(iso, workbook.date_system)?)
        }
        CellValue::Formula {
            expression,
            cached_value,
        } => {
            let value = match cached_value {
                Some(FormulaResult::Number { value }) => format!("<v>{value}</v>"),
                Some(FormulaResult::Text { value }) => {
                    attributes.push_str(" t=\"str\"");
                    format!("<v>{}</v>", xml_text(value))
                }
                Some(FormulaResult::Boolean { value }) => {
                    attributes.push_str(" t=\"b\"");
                    format!("<v>{}</v>", u8::from(*value))
                }
                Some(FormulaResult::Error { code }) => {
                    attributes.push_str(" t=\"e\"");
                    format!("<v>{}</v>", xml_text(code))
                }
                None => String::new(),
            };
            format!(
                "<f>{}</f>{value}",
                xml_text(expression.strip_prefix('=').unwrap_or(expression))
            )
        }
    };
    if let Some(style) = style {
        attributes.push_str(&format!(" s=\"{style}\""));
    }
    Ok(format!("<c{attributes}>{body}</c>"))
}

fn excel_date_serial(value: &str, date_system: WorkbookDateSystem) -> Result<f64, String> {
    let (date, seconds) = if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        (date, 0.0)
    } else {
        let date_time = DateTime::parse_from_rfc3339(value)
            .map_err(|_| format!("Invalid ISO date {value}."))?;
        (
            date_time.date_naive(),
            date_time.time().num_seconds_from_midnight() as f64
                + f64::from(date_time.timestamp_subsec_nanos()) / 1_000_000_000.0,
        )
    };
    let whole_days = match date_system {
        WorkbookDateSystem::Excel1900 => {
            let epoch = NaiveDate::from_ymd_opt(1899, 12, 31).unwrap();
            if date <= epoch {
                return Err("Excel 1900 dates must be on or after 1900-01-01.".to_string());
            }
            let days = (date - epoch).num_days();
            if date >= NaiveDate::from_ymd_opt(1900, 3, 1).unwrap() {
                days + 1
            } else {
                days
            }
        }
        WorkbookDateSystem::Excel1904 => {
            let epoch = NaiveDate::from_ymd_opt(1904, 1, 1).unwrap();
            if date < epoch {
                return Err("Excel 1904 dates must be on or after 1904-01-01.".to_string());
            }
            (date - epoch).num_days()
        }
    };
    Ok(whole_days as f64 + seconds / 86_400.0)
}

fn validation_xml(validation: &super::DataValidation) -> Result<String, String> {
    let (kind, operator, first, second) = match &validation.rule {
        ValidationRule::List { values } => (
            "list",
            "",
            format!(
                "\"{}\"",
                values
                    .iter()
                    .map(|value| value.replace('"', "\"\""))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            None,
        ),
        ValidationRule::WholeNumber { minimum, maximum } => (
            "whole",
            "between",
            minimum.to_string(),
            Some(maximum.to_string()),
        ),
        ValidationRule::Decimal { minimum, maximum } => (
            "decimal",
            "between",
            minimum.to_string(),
            Some(maximum.to_string()),
        ),
        ValidationRule::Date {
            minimum_iso,
            maximum_iso,
        } => (
            "date",
            "between",
            date_formula(minimum_iso)?,
            Some(date_formula(maximum_iso)?),
        ),
        ValidationRule::CustomFormula { formula } => (
            "custom",
            "",
            formula.strip_prefix('=').unwrap_or(formula).to_string(),
            None,
        ),
    };
    let mut attrs = format!(
        " type=\"{kind}\" sqref=\"{}\" allowBlank=\"{}\"",
        xml_attr(&validation.range),
        u8::from(validation.allow_blank)
    );
    if !operator.is_empty() {
        attrs.push_str(&format!(" operator=\"{operator}\""));
    }
    if let Some(prompt) = &validation.prompt {
        attrs.push_str(&format!(
            " prompt=\"{}\" showInputMessage=\"1\"",
            xml_attr(prompt)
        ));
    }
    if let Some(error) = &validation.error {
        attrs.push_str(&format!(
            " error=\"{}\" showErrorMessage=\"1\"",
            xml_attr(error)
        ));
    }
    Ok(format!(
        "<dataValidation{attrs}><formula1>{}</formula1>{}</dataValidation>",
        xml_text(&first),
        second
            .map(|value| format!("<formula2>{}</formula2>", xml_text(&value)))
            .unwrap_or_default()
    ))
}

fn date_formula(value: &str) -> Result<String, String> {
    let date = NaiveDate::parse_from_str(value.get(..10).unwrap_or(value), "%Y-%m-%d")
        .map_err(|_| "Validation date must begin with YYYY-MM-DD.".to_string())?;
    Ok(format!(
        "DATE({},{},{})",
        date.format("%Y"),
        date.format("%m"),
        date.format("%d")
    ))
}

fn table_xml(table: &super::WorkbookTable, table_id: u32) -> Result<String, String> {
    parse_local_range(&table.range)?;
    let columns = table
        .columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            format!(
                "<tableColumn id=\"{}\" name=\"{}\"/>",
                index + 1,
                xml_attr(column)
            )
        })
        .collect::<String>();
    Ok(format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><table xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" id=\"{table_id}\" name=\"{}\" displayName=\"{}\" ref=\"{}\" totalsRowShown=\"0\"><autoFilter ref=\"{}\"/><tableColumns count=\"{}\">{columns}</tableColumns><tableStyleInfo name=\"{}\" showFirstColumn=\"0\" showLastColumn=\"0\" showRowStripes=\"1\" showColumnStripes=\"0\"/></table>", xml_attr(&table.name), xml_attr(&table.name), xml_attr(&table.range), xml_attr(&table.range), table.columns.len(), xml_attr(&table.style)))
}

fn drawing_parts(
    workbook: &WorkbookIr,
    sheet: &Worksheet,
    sheet_index: usize,
    next_chart_id: &mut u32,
    cell_index: &WorkbookCellIndex<'_>,
) -> Result<(String, String, Vec<(String, String)>), String> {
    let mut drawing = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><xdr:wsDr xmlns:xdr=\"http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">");
    let mut rels = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">");
    let mut charts = Vec::new();
    for (index, chart) in sheet.charts.iter().enumerate() {
        let chart_id = *next_chart_id;
        *next_chart_id += 1;
        let relationship = index + 1;
        rels.push_str(&format!("<Relationship Id=\"rId{relationship}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart\" Target=\"../charts/chart{chart_id}.xml\"/>"));
        drawing.push_str(&format!("<xdr:twoCellAnchor><xdr:from><xdr:col>{}</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>{}</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>{}</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>{}</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to><xdr:graphicFrame macro=\"\"><xdr:nvGraphicFramePr><xdr:cNvPr id=\"{}\" name=\"Chart {}\"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm/><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/chart\"><c:chart xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\" r:id=\"rId{relationship}\"/></a:graphicData></a:graphic></xdr:graphicFrame><xdr:clientData/></xdr:twoCellAnchor>", chart.anchor.from_column, chart.anchor.from_row, chart.anchor.to_column, chart.anchor.to_row, index + 2, index + 1));
        charts.push((
            format!("xl/charts/chart{chart_id}.xml"),
            chart_xml(workbook, chart, &sheet.name, cell_index)?,
        ));
    }
    drawing.push_str("</xdr:wsDr>");
    rels.push_str("</Relationships>");
    let _ = sheet_index;
    Ok((drawing, rels, charts))
}

fn chart_xml(
    workbook: &WorkbookIr,
    chart: &super::WorkbookChart,
    current_sheet: &str,
    cell_index: &WorkbookCellIndex<'_>,
) -> Result<String, String> {
    let chart_tag = match chart.kind {
        ChartKind::Line => "lineChart",
        ChartKind::Bar | ChartKind::Column => "barChart",
    };
    let mut body = format!("<c:{chart_tag}>");
    if matches!(chart.kind, ChartKind::Line) {
        body.push_str("<c:grouping val=\"standard\"/>");
    } else {
        body.push_str(&format!(
            "<c:barDir val=\"{}\"/><c:grouping val=\"clustered\"/><c:varyColors val=\"0\"/>",
            if matches!(chart.kind, ChartKind::Bar) {
                "bar"
            } else {
                "col"
            }
        ));
    }
    let categories = chart_points(
        workbook,
        &chart.category_range,
        current_sheet,
        false,
        cell_index,
    )?;
    for (index, series) in chart.series.iter().enumerate() {
        const SERIES_COLORS: [&str; 6] =
            ["4472C4", "70AD47", "ED7D31", "5B9BD5", "A5A5A5", "FFC000"];
        let color = SERIES_COLORS[index % SERIES_COLORS.len()];
        let values = chart_points(
            workbook,
            &series.value_range,
            current_sheet,
            true,
            cell_index,
        )?;
        body.push_str(&format!("<c:ser><c:idx val=\"{index}\"/><c:order val=\"{index}\"/><c:tx><c:v>{}</c:v></c:tx><c:spPr><a:solidFill><a:srgbClr val=\"{color}\"/></a:solidFill><a:ln><a:noFill/></a:ln></c:spPr><c:cat><c:strRef><c:f>{}</c:f>{}</c:strRef></c:cat><c:val><c:numRef><c:f>{}</c:f>{}</c:numRef></c:val></c:ser>", xml_text(&series.name), xml_text(&normalized_range(&chart.category_range, current_sheet)?), string_cache(&categories), xml_text(&normalized_range(&series.value_range, current_sheet)?), number_cache(&values)));
    }
    if matches!(chart.kind, ChartKind::Line) {
        body.push_str("<c:marker val=\"1\"/>");
    }
    body.push_str("<c:axId val=\"48650112\"/><c:axId val=\"48672768\"/>");
    body.push_str(&format!("</c:{chart_tag}>"));
    Ok(format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><c:chartSpace xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><c:chart><c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang=\"{}\"/><a:t>{}</a:t></a:r></a:p></c:rich></c:tx><c:layout/></c:title><c:autoTitleDeleted val=\"0\"/><c:plotArea><c:layout/>{body}<c:catAx><c:axId val=\"48650112\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:axPos val=\"b\"/><c:crossAx val=\"48672768\"/><c:crosses val=\"autoZero\"/></c:catAx><c:valAx><c:axId val=\"48672768\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:axPos val=\"l\"/><c:crossAx val=\"48650112\"/><c:crosses val=\"autoZero\"/></c:valAx></c:plotArea><c:legend><c:legendPos val=\"b\"/><c:layout/><c:overlay val=\"0\"/></c:legend><c:plotVisOnly val=\"1\"/></c:chart></c:chartSpace>", xml_attr(&workbook.locale), xml_text(&chart.title)))
}

fn chart_points(
    workbook: &WorkbookIr,
    raw: &str,
    current_sheet: &str,
    numeric: bool,
    cell_index: &WorkbookCellIndex<'_>,
) -> Result<Vec<Option<String>>, String> {
    let (sheet_name, range) = split_qualified_range(raw, current_sheet)?;
    let sheet = workbook
        .worksheets
        .iter()
        .find(|sheet| sheet.name.eq_ignore_ascii_case(&sheet_name))
        .ok_or_else(|| format!("Chart range references missing worksheet {sheet_name}."))?;
    let mut points = Vec::with_capacity(range.cell_count() as usize);
    for row in range.start.row..=range.end.row {
        for column in range.start.column..=range.end.column {
            let value = cell_index
                .get(&(sheet.name.to_lowercase(), row, column))
                .and_then(|value| chart_cell_value(value, numeric));
            points.push(value);
        }
    }
    Ok(points)
}

fn chart_cell_value(value: &CellValue, numeric: bool) -> Option<String> {
    if numeric {
        return match value {
            CellValue::Number { value } => Some(value.to_string()),
            CellValue::Formula {
                cached_value: Some(FormulaResult::Number { value }),
                ..
            } => Some(value.to_string()),
            _ => None,
        };
    }
    match value {
        CellValue::Text { value } => Some(value.clone()),
        CellValue::Number { value } => Some(value.to_string()),
        CellValue::Boolean { value } => Some(if *value { "TRUE" } else { "FALSE" }.to_string()),
        CellValue::Date { iso } => Some(iso.clone()),
        CellValue::Formula {
            cached_value: Some(FormulaResult::Number { value }),
            ..
        } => Some(value.to_string()),
        CellValue::Formula {
            cached_value: Some(FormulaResult::Text { value }),
            ..
        } => Some(value.clone()),
        CellValue::Formula {
            cached_value: Some(FormulaResult::Boolean { value }),
            ..
        } => Some(if *value { "TRUE" } else { "FALSE" }.to_string()),
        _ => None,
    }
}

fn string_cache(points: &[Option<String>]) -> String {
    let values = points
        .iter()
        .enumerate()
        .filter_map(|(index, value)| {
            value.as_ref().map(|value| {
                format!(
                    "<c:pt idx=\"{index}\"><c:v>{}</c:v></c:pt>",
                    xml_text(value)
                )
            })
        })
        .collect::<String>();
    format!(
        "<c:strCache><c:ptCount val=\"{}\"/>{values}</c:strCache>",
        points.len()
    )
}
fn number_cache(points: &[Option<String>]) -> String {
    let values = points
        .iter()
        .enumerate()
        .filter_map(|(index, value)| {
            value.as_ref().map(|value| {
                format!(
                    "<c:pt idx=\"{index}\"><c:v>{}</c:v></c:pt>",
                    xml_text(value)
                )
            })
        })
        .collect::<String>();
    format!("<c:numCache><c:formatCode>General</c:formatCode><c:ptCount val=\"{}\"/>{values}</c:numCache>",points.len())
}

fn normalized_range(raw: &str, current_sheet: &str) -> Result<String, String> {
    let (sheet, range) = split_qualified_range(raw, current_sheet)?;
    Ok(format!(
        "{}!${}${}:${}${}",
        quote_sheet_name(&sheet),
        super::address::column_name(range.start.column),
        range.start.row,
        super::address::column_name(range.end.column),
        range.end.row
    ))
}

fn comments_xml(comments: &[(&super::WorkbookCell, &super::CellComment)]) -> String {
    let mut authors = Vec::new();
    for (_, comment) in comments {
        if !authors.contains(&comment.author) {
            authors.push(comment.author.clone());
        }
    }
    let author_xml = authors
        .iter()
        .map(|author| format!("<author>{}</author>", xml_text(author)))
        .collect::<String>();
    let comment_xml = comments.iter().map(|(cell, comment)| format!("<comment ref=\"{}\" authorId=\"{}\"><text><r><rPr><sz val=\"9\"/><rFont val=\"Arial\"/></rPr><t xml:space=\"preserve\">{}</t></r></text></comment>", xml_attr(&cell.address), authors.iter().position(|value| value == &comment.author).unwrap_or(0), xml_text(&comment.text))).collect::<String>();
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><comments xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><authors>{author_xml}</authors><commentList>{comment_xml}</commentList></comments>")
}

fn comments_vml(
    comments: &[(&super::WorkbookCell, &super::CellComment)],
) -> Result<String, String> {
    let mut shapes = String::new();
    for (index, (cell, _)) in comments.iter().enumerate() {
        let address = parse_cell_address(&cell.address)?;
        shapes.push_str(&format!("<v:shape id=\"_x0000_s{}\" type=\"#_x0000_t202\" style=\"position:absolute;margin-left:80pt;margin-top:5pt;width:144pt;height:79pt;z-index:1;visibility:hidden\" fillcolor=\"#ffffe1\" o:insetmode=\"auto\"><v:fill color2=\"#ffffe1\"/><v:shadow on=\"t\" color=\"black\" obscured=\"t\"/><v:path o:connecttype=\"none\"/><v:textbox style=\"mso-direction-alt:auto\"><div style=\"text-align:left\"/></v:textbox><x:ClientData ObjectType=\"Note\"><x:MoveWithCells/><x:SizeWithCells/><x:Anchor>{}, 15, {}, 2, {}, 31, {}, 1</x:Anchor><x:AutoFill>False</x:AutoFill><x:Row>{}</x:Row><x:Column>{}</x:Column></x:ClientData></v:shape>", 1025 + index, address.column.saturating_sub(1), address.row.saturating_sub(1), address.column + 2, address.row + 4, address.row - 1, address.column - 1));
    }
    Ok(format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><xml xmlns:v=\"urn:schemas-microsoft-com:vml\" xmlns:o=\"urn:schemas-microsoft-com:office:office\" xmlns:x=\"urn:schemas-microsoft-com:office:excel\"><o:shapelayout v:ext=\"edit\"><o:idmap v:ext=\"edit\" data=\"1\"/></o:shapelayout><v:shapetype id=\"_x0000_t202\" coordsize=\"21600,21600\" o:spt=\"202\" path=\"m,l,21600r21600,l21600,xe\"><v:stroke joinstyle=\"miter\"/><v:path gradientshapeok=\"t\" o:connecttype=\"rect\"/></v:shapetype>{shapes}</xml>"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifacts::workbooks::deterministic_fixture;

    fn fixture_sheet_xml(workbook: &WorkbookIr, sheet_index: usize) -> String {
        let styles = super::super::style_xml::build_styles(workbook).unwrap();
        worksheet_xml(
            workbook,
            &workbook.worksheets[sheet_index],
            &styles,
            &[],
            None,
            None,
        )
        .unwrap()
    }

    #[test]
    fn excel_date_serials_honor_the_1900_compatibility_discontinuity() {
        assert_eq!(
            excel_date_serial("1900-01-01", WorkbookDateSystem::Excel1900).unwrap(),
            1.0
        );
        assert_eq!(
            excel_date_serial("1900-02-28", WorkbookDateSystem::Excel1900).unwrap(),
            59.0
        );
        assert_eq!(
            excel_date_serial("1900-03-01", WorkbookDateSystem::Excel1900).unwrap(),
            61.0
        );
        assert_eq!(
            excel_date_serial("1904-01-01", WorkbookDateSystem::Excel1904).unwrap(),
            0.0
        );
        assert!(excel_date_serial("1899-12-31", WorkbookDateSystem::Excel1900).is_err());
    }

    #[test]
    fn worksheets_emit_a_canonical_fit_to_page_print_contract() {
        let workbook = deterministic_fixture().unwrap();
        let xml = fixture_sheet_xml(&workbook, 0);

        assert!(
            xml.contains("<sheetPr><pageSetUpPr fitToPage=\"1\" autoPageBreaks=\"0\"/></sheetPr>")
        );
        assert!(
            xml.contains("<printOptions horizontalCentered=\"1\" gridLines=\"0\" headings=\"0\"/>")
        );
        assert!(xml.contains("<pageMargins left=\"0.25\" right=\"0.25\" top=\"0.50\" bottom=\"0.50\" header=\"0.20\" footer=\"0.20\"/>"));
        assert!(xml.contains("<pageSetup paperSize=\"1\" orientation=\"landscape\" fitToWidth=\"1\" fitToHeight=\"0\" firstPageNumber=\"1\" useFirstPageNumber=\"1\"/>"));
        assert!(xml.find("<pageMargins").unwrap() < xml.find("<pageSetup").unwrap());
        assert!(xml.find("<pageSetup").unwrap() < xml.find("</worksheet>").unwrap());
    }

    #[test]
    fn charts_emit_visible_series_fills_and_a_legend() {
        let workbook = deterministic_fixture().unwrap();
        let index = workbook_cell_index(&workbook).unwrap();
        let chart = chart_xml(
            &workbook,
            &workbook.worksheets[0].charts[0],
            &workbook.worksheets[0].name,
            &index,
        )
        .unwrap();

        assert!(chart.contains("<a:srgbClr val=\"4472C4\"/>"));
        assert!(chart.contains("<c:legend><c:legendPos val=\"b\"/>"));
    }
}
