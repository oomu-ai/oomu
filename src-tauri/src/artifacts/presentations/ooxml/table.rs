use super::{bool_u8, text_body};
use crate::artifacts::presentations::{xml, PresentationElement, PresentationTable, TextBlock};

const DARK_HEADER_FILL: &str = "17365D";
const LIGHT_HEADER_FILL: &str = "D9E2F3";

pub(super) fn table_shape(
    id: usize,
    element: &PresentationElement,
    table: &PresentationTable,
) -> Result<String, String> {
    let frame = element.frame;
    let columns = table.rows[0].len();
    let grid = (0..columns)
        .map(|_| format!(r#"<a:gridCol w="{}"/>"#, frame.width / columns as i64))
        .collect::<String>();
    let rows = table
        .rows
        .iter()
        .enumerate()
        .map(|(row_index, row)| {
            let cells = row
                .iter()
                .map(|cell| {
                    let properties = cell_properties(cell, table.header_row && row_index == 0);
                    Ok(format!(
                        r#"<a:tc>{}{properties}</a:tc>"#,
                        text_body(cell)?.replace("p:txBody", "a:txBody"),
                    ))
                })
                .collect::<Result<String, String>>()?;
            Ok(format!(
                r#"<a:tr h="{}">{cells}</a:tr>"#,
                frame.height / table.rows.len() as i64
            ))
        })
        .collect::<Result<String, String>>()?;
    Ok(format!(
        r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="{id}" name="{}"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="{}" y="{}"/><a:ext cx="{}" cy="{}"/></p:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table"><a:tbl><a:tblPr firstRow="{}" bandRow="1"><a:tableStyleId>{{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}}</a:tableStyleId></a:tblPr><a:tblGrid>{grid}</a:tblGrid>{rows}</a:tbl></a:graphicData></a:graphic></p:graphicFrame>"#,
        xml::attr(&element.object_id),
        frame.x,
        frame.y,
        frame.width,
        frame.height,
        bool_u8(table.header_row)
    ))
}

fn cell_properties(cell: &TextBlock, is_header: bool) -> String {
    if !is_header {
        return "<a:tcPr/>".to_string();
    }
    let fill = if header_uses_light_text(cell) {
        DARK_HEADER_FILL
    } else {
        LIGHT_HEADER_FILL
    };
    format!(r#"<a:tcPr><a:solidFill><a:srgbClr val="{fill}"/></a:solidFill></a:tcPr>"#)
}

fn header_uses_light_text(cell: &TextBlock) -> bool {
    cell.paragraphs
        .iter()
        .flat_map(|paragraph| &paragraph.runs)
        .next()
        .and_then(|run| u32::from_str_radix(&run.color, 16).ok())
        .is_some_and(|rgb| {
            let red = (rgb >> 16) & 0xff;
            let green = (rgb >> 8) & 0xff;
            let blue = rgb & 0xff;
            red * 299 + green * 587 + blue * 114 >= 160_000
        })
}
