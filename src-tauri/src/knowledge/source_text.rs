use regex::Regex;
use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

pub(super) fn read_bounded_granted_file(
    handle: &mut File,
    max_bytes: u64,
) -> Result<Vec<u8>, super::KnowledgeError> {
    handle
        .seek(SeekFrom::Start(0))
        .map_err(super::KnowledgeError::io)?;
    let mut bytes = Vec::new();
    handle
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(super::KnowledgeError::io)?;
    handle
        .seek(SeekFrom::Start(0))
        .map_err(super::KnowledgeError::io)?;
    if bytes.len() as u64 > max_bytes {
        return Err(super::KnowledgeError::grant(
            "Knowledge file exceeded the per-file byte limit.",
        ));
    }
    Ok(bytes)
}

pub(super) fn knowledge_source_byte_limit(path: &Path) -> u64 {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("docx" | "pdf" | "xlsx") => super::MAX_BINARY_SOURCE_BYTES,
        _ => super::MAX_FILE_BYTES,
    }
}

pub(super) fn push_token_bounded_line_chunks(
    chunks: &mut Vec<(usize, usize, String)>,
    lines: &[&str],
    start: usize,
    end: usize,
) {
    let max_chars = super::MAX_CHUNK_TOKENS.saturating_mul(4).max(1);
    let mut cursor = start;
    while cursor < end {
        let line_chars = lines[cursor].chars().count();
        if line_chars > max_chars {
            let characters = lines[cursor].chars().collect::<Vec<_>>();
            for part in characters.chunks(max_chars) {
                chunks.push((cursor + 1, cursor + 1, part.iter().collect()));
            }
            cursor += 1;
            continue;
        }

        let mut chunk_end = cursor;
        let mut used_chars = 0;
        while chunk_end < end {
            let separator_chars = usize::from(chunk_end > cursor);
            let next_chars = lines[chunk_end].chars().count();
            if chunk_end > cursor && used_chars + separator_chars + next_chars > max_chars {
                break;
            }
            used_chars += separator_chars + next_chars;
            chunk_end += 1;
        }
        chunks.push((cursor + 1, chunk_end, lines[cursor..chunk_end].join("\n")));
        cursor = chunk_end;
    }
}

pub(crate) fn extract_file_text(path: &Path, bytes: &[u8]) -> Result<String, String> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let text = match extension.as_str() {
        "docx" => extract_docx_text(bytes)?,
        "pdf" => crate::pdf_containment::extract_pdf_bytes_contained(bytes)
            .map(|result| result.text)
            .map_err(|_| "A Project PDF could not be read safely.".to_string())?,
        "xlsx" => extract_xlsx_text(bytes)?,
        _ => String::from_utf8(bytes.to_vec())
            .map_err(|_| "A Project source is not readable text.".to_string())?,
    };
    Ok(bounded_text(text))
}

fn extract_docx_text(bytes: &[u8]) -> Result<String, String> {
    let entries = crate::foundation::office_zip::read_zip(bytes)
        .map_err(|_| "A Project Word document could not be read safely.".to_string())?;
    let document = entries
        .get("word/document.xml")
        .ok_or_else(|| "A Project Word document is missing its document body.".to_string())?;
    let xml = std::str::from_utf8(document)
        .map_err(|_| "A Project Word document body is not readable XML.".to_string())?;
    let token_pattern = Regex::new(
        r#"(?s)<w:t(?:\s[^>]*)?>(.*?)</w:t>|<w:tab(?:\s[^>]*)?/?>|<w:br(?:\s[^>]*)?/?>|</w:p\s*>|</w:tr\s*>"#,
    )
    .map_err(|error| error.to_string())?;
    let mut text = String::new();
    for captures in token_pattern.captures_iter(xml) {
        let token = captures
            .get(0)
            .map(|value| value.as_str())
            .unwrap_or_default();
        if let Some(value) = captures.get(1) {
            text.push_str(&xml_unescape(value.as_str()));
        } else if token.starts_with("<w:tab") {
            text.push('\t');
        } else if token.starts_with("<w:br") {
            text.push('\n');
        } else if !text.ends_with('\n') {
            text.push('\n');
        }
    }
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("A Project Word document has no readable text.".to_string());
    }
    Ok(text)
}

fn extract_xlsx_text(bytes: &[u8]) -> Result<String, String> {
    let entries = crate::foundation::office_zip::read_zip(bytes)
        .map_err(|_| "A Project workbook could not be read safely.".to_string())?;
    let shared_strings = entries
        .get("xl/sharedStrings.xml")
        .map(|bytes| parse_shared_strings(bytes))
        .transpose()?
        .unwrap_or_default();
    let sheets = workbook_sheets(&entries)?;
    let mut output = String::from("Workbook cell data with original cell addresses:\n");
    let mut readable_cells = 0_usize;
    for (sheet_name, part) in sheets {
        let Some(bytes) = entries.get(&part) else {
            continue;
        };
        let xml = std::str::from_utf8(bytes)
            .map_err(|_| "A Project worksheet is not readable XML.".to_string())?;
        let cells = worksheet_cells(xml, &shared_strings)?;
        if cells.is_empty() {
            continue;
        }
        output.push_str(&format!("\n[WORKSHEET name=\"{}\"]\n", sheet_name));
        let mut active_row = String::new();
        for (address, value) in cells {
            let row = address
                .chars()
                .skip_while(|character| character.is_ascii_alphabetic())
                .collect::<String>();
            if row != active_row {
                if !active_row.is_empty() {
                    output.push('\n');
                }
                output.push_str(&format!("row {row}: "));
                active_row = row;
            } else {
                output.push_str(" | ");
            }
            output.push_str(&format!("{address}={value}"));
            readable_cells += 1;
        }
        output.push_str("\n[/WORKSHEET]\n");
    }
    if readable_cells == 0 {
        return Err("A Project workbook has no readable cell values.".to_string());
    }
    Ok(output)
}

fn workbook_sheets(entries: &BTreeMap<String, Vec<u8>>) -> Result<Vec<(String, String)>, String> {
    let workbook = utf8_entry(entries, "xl/workbook.xml")?;
    let relationships = utf8_entry(entries, "xl/_rels/workbook.xml.rels")?;
    let relationship_pattern =
        Regex::new(r#"<Relationship\b[^>]*>"#).map_err(|error| error.to_string())?;
    let mut targets = HashMap::new();
    for relationship in relationship_pattern.find_iter(relationships) {
        let tag = relationship.as_str();
        let (Some(id), Some(target)) = (attribute(tag, "Id"), attribute(tag, "Target")) else {
            continue;
        };
        let target = target.trim_start_matches('/').replace('\\', "/");
        if target.split('/').any(|part| part == "..") {
            continue;
        }
        let part = if target.starts_with("xl/") {
            target
        } else {
            format!("xl/{target}")
        };
        targets.insert(id, part);
    }
    let sheet_pattern = Regex::new(r#"<sheet\b[^>]*>"#).map_err(|error| error.to_string())?;
    let sheets = sheet_pattern
        .find_iter(workbook)
        .filter_map(|sheet| {
            let tag = sheet.as_str();
            let name = attribute(tag, "name")?;
            let relationship_id = attribute(tag, "r:id")?;
            targets
                .get(&relationship_id)
                .cloned()
                .map(|part| (xml_unescape(&name), part))
        })
        .collect::<Vec<_>>();
    if sheets.is_empty() {
        return Err("A Project workbook has no readable worksheets.".to_string());
    }
    Ok(sheets)
}

fn parse_shared_strings(bytes: &[u8]) -> Result<Vec<String>, String> {
    let xml = std::str::from_utf8(bytes)
        .map_err(|_| "Project workbook strings are not readable XML.".to_string())?;
    let item_pattern =
        Regex::new(r"(?s)<si\b[^>]*>(.*?)</si>").map_err(|error| error.to_string())?;
    item_pattern
        .captures_iter(xml)
        .map(|capture| text_nodes(&capture[1]))
        .collect()
}

fn worksheet_cells(xml: &str, shared_strings: &[String]) -> Result<Vec<(String, String)>, String> {
    let cell_pattern =
        Regex::new(r"(?s)<c\b([^>]*)>(.*?)</c>").map_err(|error| error.to_string())?;
    let value_pattern =
        Regex::new(r"(?s)<v\b[^>]*>(.*?)</v>").map_err(|error| error.to_string())?;
    let mut cells = Vec::new();
    for cell in cell_pattern.captures_iter(xml) {
        let tag = format!("<c{}>", &cell[1]);
        let Some(address) = attribute(&tag, "r") else {
            continue;
        };
        let kind = attribute(&tag, "t").unwrap_or_default();
        let value = if kind == "inlineStr" {
            text_nodes(&cell[2])?
        } else {
            let Some(raw) = value_pattern
                .captures(&cell[2])
                .and_then(|capture| capture.get(1))
                .map(|value| xml_unescape(value.as_str()))
            else {
                continue;
            };
            match kind.as_str() {
                "s" => raw
                    .parse::<usize>()
                    .ok()
                    .and_then(|index| shared_strings.get(index).cloned())
                    .unwrap_or(raw),
                "b" if raw == "1" => "true".to_string(),
                "b" if raw == "0" => "false".to_string(),
                _ => raw,
            }
        };
        if !value.trim().is_empty() {
            cells.push((address, value));
        }
    }
    Ok(cells)
}

fn text_nodes(xml: &str) -> Result<String, String> {
    let text_pattern = Regex::new(r"(?s)<t\b[^>]*>(.*?)</t>").map_err(|error| error.to_string())?;
    Ok(text_pattern
        .captures_iter(xml)
        .filter_map(|capture| capture.get(1))
        .map(|value| xml_unescape(value.as_str()))
        .collect::<String>())
}

fn utf8_entry<'a>(entries: &'a BTreeMap<String, Vec<u8>>, path: &str) -> Result<&'a str, String> {
    std::str::from_utf8(
        entries
            .get(path)
            .ok_or_else(|| "A Project workbook is missing required structure.".to_string())?,
    )
    .map_err(|_| "A Project workbook structure is not readable XML.".to_string())
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let pattern = Regex::new(&format!(r#"(?:^|\s){}=\"([^\"]*)\""#, regex::escape(name))).ok()?;
    pattern
        .captures(tag)
        .and_then(|capture| capture.get(1))
        .map(|value| value.as_str().to_string())
}

fn xml_unescape(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn bounded_text(mut text: String) -> String {
    let max_bytes = super::MAX_FILE_BYTES as usize;
    if text.len() <= max_bytes {
        return text;
    }
    let mut boundary = max_bytes;
    while !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    text.truncate(boundary);
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_shared_inline_and_numeric_workbook_cells() {
        let entries = BTreeMap::from([
            ("[Content_Types].xml".to_string(), b"<Types/>".to_vec()),
            ("xl/workbook.xml".to_string(), br#"<workbook><sheets><sheet name="Inventory" r:id="rId1"/></sheets></workbook>"#.to_vec()),
            ("xl/_rels/workbook.xml.rels".to_string(), br#"<Relationships><Relationship Id="rId1" Target="worksheets/sheet1.xml"/></Relationships>"#.to_vec()),
            ("xl/sharedStrings.xml".to_string(), br#"<sst><si><t>WS-LAB-001</t></si><si><t>Ready</t></si></sst>"#.to_vec()),
            ("xl/worksheets/sheet1.xml".to_string(), br#"<worksheet><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c><c r="C1"><v>42</v></c><c r="D1" t="inlineStr"><is><t>Calibrated</t></is></c></row></sheetData></worksheet>"#.to_vec()),
        ]);
        let bytes = crate::foundation::office_zip::write_store_zip(&entries).unwrap();
        let text = extract_xlsx_text(&bytes).unwrap();
        assert!(text.contains("[WORKSHEET name=\"Inventory\"]"));
        assert!(text.contains("A1=WS-LAB-001"));
        assert!(text.contains("B1=Ready"));
        assert!(text.contains("C1=42"));
        assert!(text.contains("D1=Calibrated"));
    }

    #[test]
    fn extracts_bounded_word_document_text() {
        let entries = BTreeMap::from([
            ("[Content_Types].xml".to_string(), b"<Types/>".to_vec()),
            ("word/document.xml".to_string(), br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Hello &amp; welcome</w:t></w:r><w:r><w:tab/></w:r><w:r><w:t>team</w:t></w:r></w:p><w:p><w:r><w:t>Second paragraph</w:t></w:r></w:p></w:body></w:document>"#.to_vec()),
        ]);
        let bytes = crate::foundation::office_zip::write_store_zip(&entries).unwrap();
        let text = extract_docx_text(&bytes).unwrap();
        assert_eq!(text, "Hello & welcome\tteam\nSecond paragraph");
    }
}
