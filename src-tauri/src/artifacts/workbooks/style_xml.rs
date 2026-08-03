use super::{CellAlignment, WorkbookIr};
use std::collections::HashMap;

pub(crate) struct StyleCatalog {
    pub xml: Vec<u8>,
    pub indexes: HashMap<String, u32>,
    pub date_style_index: u32,
}

pub(crate) fn build_styles(workbook: &WorkbookIr) -> Result<StyleCatalog, String> {
    let mut number_formats = Vec::new();
    for format in &workbook.formats {
        if let Some(code) = &format.number_format {
            if !number_formats
                .iter()
                .any(|existing: &String| existing == code)
            {
                number_formats.push(code.clone());
            }
        }
    }
    let mut fonts = vec![FontKey::default()];
    let mut fills = vec![None, Some("gray125".to_string())];
    for format in &workbook.formats {
        let font = FontKey {
            bold: format.font.bold,
            italic: format.font.italic,
            color: format.font.color.clone(),
            size: format.font.size_pt,
        };
        if !fonts.contains(&font) {
            fonts.push(font);
        }
        if let Some(color) = &format.fill_color {
            let value = Some(color.clone());
            if !fills.contains(&value) {
                fills.push(value);
            }
        }
    }
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">");
    if !number_formats.is_empty() {
        xml.push_str(&format!("<numFmts count=\"{}\">", number_formats.len()));
        for (index, value) in number_formats.iter().enumerate() {
            xml.push_str(&format!(
                "<numFmt numFmtId=\"{}\" formatCode=\"{}\"/>",
                164 + index,
                xml_attr(value)
            ));
        }
        xml.push_str("</numFmts>");
    }
    xml.push_str(&format!("<fonts count=\"{}\">", fonts.len()));
    for font in &fonts {
        xml.push_str("<font>");
        if font.bold {
            xml.push_str("<b/>");
        }
        if font.italic {
            xml.push_str("<i/>");
        }
        xml.push_str(&format!("<sz val=\"{}\"/>", font.size.unwrap_or(11.0)));
        if let Some(color) = &font.color {
            xml.push_str(&format!("<color rgb=\"FF{}\"/>", xml_attr(color)));
        } else {
            xml.push_str("<color theme=\"1\"/>");
        }
        xml.push_str("<name val=\"Arial\"/><family val=\"2\"/><scheme val=\"minor\"/></font>");
    }
    xml.push_str("</fonts>");
    xml.push_str(&format!("<fills count=\"{}\">", fills.len()));
    for fill in &fills {
        match fill.as_deref() {
            None => xml.push_str("<fill><patternFill patternType=\"none\"/></fill>"),
            Some("gray125") => xml.push_str("<fill><patternFill patternType=\"gray125\"/></fill>"),
            Some(color) => xml.push_str(&format!("<fill><patternFill patternType=\"solid\"><fgColor rgb=\"FF{}\"/><bgColor indexed=\"64\"/></patternFill></fill>", xml_attr(color))),
        }
    }
    xml.push_str("</fills><borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs>");
    let date_style_index = workbook.formats.len() as u32 + 1;
    xml.push_str(&format!("<cellXfs count=\"{}\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/>", workbook.formats.len() + 2));
    let mut indexes = HashMap::new();
    for (index, format) in workbook.formats.iter().enumerate() {
        indexes.insert(format.format_id.clone(), index as u32 + 1);
        let font_key = FontKey {
            bold: format.font.bold,
            italic: format.font.italic,
            color: format.font.color.clone(),
            size: format.font.size_pt,
        };
        let font_id = fonts
            .iter()
            .position(|value| value == &font_key)
            .unwrap_or(0);
        let fill_id = format
            .fill_color
            .as_ref()
            .and_then(|color| fills.iter().position(|value| value.as_ref() == Some(color)))
            .unwrap_or(0);
        let num_fmt = format
            .number_format
            .as_ref()
            .and_then(|code| number_formats.iter().position(|value| value == code))
            .map(|value| 164 + value)
            .unwrap_or(0);
        let apply_alignment =
            !matches!(format.alignment, CellAlignment::General) || format.wrap_text;
        xml.push_str(&format!("<xf numFmtId=\"{num_fmt}\" fontId=\"{font_id}\" fillId=\"{fill_id}\" borderId=\"0\" xfId=\"0\" applyFont=\"1\" applyFill=\"1\"{}{}>", if num_fmt > 0 { " applyNumberFormat=\"1\"" } else { "" }, if apply_alignment { " applyAlignment=\"1\"" } else { "" }));
        if apply_alignment {
            let horizontal = match format.alignment {
                CellAlignment::General => None,
                CellAlignment::Left => Some("left"),
                CellAlignment::Center => Some("center"),
                CellAlignment::Right => Some("right"),
            };
            xml.push_str("<alignment");
            if let Some(horizontal) = horizontal {
                xml.push_str(&format!(" horizontal=\"{horizontal}\""));
            }
            if format.wrap_text {
                xml.push_str(" wrapText=\"1\"");
            }
            xml.push_str("/>");
        }
        xml.push_str("</xf>");
    }
    xml.push_str("<xf numFmtId=\"14\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/></cellXfs><cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles><dxfs count=\"0\"/><tableStyles count=\"0\" defaultTableStyle=\"TableStyleMedium2\" defaultPivotStyle=\"PivotStyleLight16\"/></styleSheet>");
    Ok(StyleCatalog {
        xml: xml.into_bytes(),
        indexes,
        date_style_index,
    })
}

#[derive(Clone, Debug, Default, PartialEq)]
struct FontKey {
    bold: bool,
    italic: bool,
    color: Option<String>,
    size: Option<f32>,
}

pub(crate) fn xml_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub(crate) fn xml_attr(value: &str) -> String {
    xml_text(value)
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
