use super::{ArtifactBlock, ArtifactDocument, ParagraphStyle, ARTIFACT_BUILDER_IDENTITY};
use lopdf::{
    content::{Content, Operation},
    dictionary, Document, Object, Stream,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::Read, path::Path};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BuildRequest {
    protocol_version: u16,
    document: ArtifactDocument,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BuildResponse {
    protocol_version: u16,
    builder_identity: &'static str,
    docx_file: &'static str,
    pdf_file: &'static str,
}

pub fn run() -> i32 {
    if std::env::args().nth(1).as_deref() == Some("--probe") {
        let _ = serde_json::to_writer(
            std::io::stdout().lock(),
            &serde_json::json!({"protocolVersion":1,"builderIdentity":ARTIFACT_BUILDER_IDENTITY,"available":true}),
        );
        return 0;
    }
    match run_inner() {
        Ok(response) => {
            if serde_json::to_writer(std::io::stdout().lock(), &response).is_ok() {
                0
            } else {
                1
            }
        }
        Err(error) => {
            let _ = serde_json::to_writer(
                std::io::stdout().lock(),
                &serde_json::json!({"error":error}),
            );
            1
        }
    }
}

fn run_inner() -> Result<BuildResponse, String> {
    let mut input = Vec::new();
    std::io::stdin()
        .lock()
        .take(1024 * 1024 + 1)
        .read_to_end(&mut input)
        .map_err(|error| error.to_string())?;
    if input.is_empty() || input.len() > 1024 * 1024 {
        return Err("Artifact helper input size is invalid.".to_string());
    }
    let request: BuildRequest = serde_json::from_slice(&input)
        .map_err(|error| format!("Artifact helper request is invalid: {error}"))?;
    if request.protocol_version != 1 {
        return Err("Artifact helper protocol version is unsupported.".to_string());
    }
    super::validation::validate(&request.document)?;
    write_docx(&request.document, Path::new("artifact.docx"))?;
    write_pdf(&request.document, Path::new("artifact.pdf"))?;
    Ok(BuildResponse {
        protocol_version: 1,
        builder_identity: ARTIFACT_BUILDER_IDENTITY,
        docx_file: "artifact.docx",
        pdf_file: "artifact.pdf",
    })
}

pub(crate) fn write_docx(document: &ArtifactDocument, path: &Path) -> Result<(), String> {
    let hyperlinks = document
        .sections
        .iter()
        .flat_map(|section| section.blocks.iter())
        .filter_map(|block| {
            if let ArtifactBlock::Citation { url, .. } = block {
                Some(url.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let mut entries = BTreeMap::new();
    entries.insert(
        "[Content_Types].xml".to_string(),
        content_types(document).into_bytes(),
    );
    entries.insert("_rels/.rels".to_string(), root_relationships().into_bytes());
    entries.insert(
        "docProps/core.xml".to_string(),
        core_properties(document).into_bytes(),
    );
    entries.insert(
        "docProps/app.xml".to_string(),
        app_properties().into_bytes(),
    );
    entries.insert(
        "word/document.xml".to_string(),
        document_xml(document).into_bytes(),
    );
    entries.insert(
        "word/styles.xml".to_string(),
        styles_xml(document).into_bytes(),
    );
    entries.insert(
        "word/numbering.xml".to_string(),
        numbering_xml().into_bytes(),
    );
    entries.insert("word/settings.xml".to_string(), settings_xml().into_bytes());
    entries.insert(
        "word/_rels/document.xml.rels".to_string(),
        document_relationships(document, &hyperlinks).into_bytes(),
    );
    if let Some(header) = document.header.as_deref() {
        entries.insert(
            "word/header1.xml".to_string(),
            header_footer_xml("hdr", header).into_bytes(),
        );
    }
    if let Some(footer) = document.footer.as_deref() {
        entries.insert(
            "word/footer1.xml".to_string(),
            header_footer_xml("ftr", footer).into_bytes(),
        );
    }
    super::package::write_store_zip(path, entries)
}

fn content_types(document: &ArtifactDocument) -> String {
    let mut overrides = String::from(
        r#"<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/><Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/><Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/><Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/>"#,
    );
    if document.header.is_some() {
        overrides.push_str(r#"<Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/>"#);
    }
    if document.footer.is_some() {
        overrides.push_str(r#"<Override PartName="/word/footer1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/>"#);
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>{overrides}</Types>"#
    )
}

fn root_relationships() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/></Relationships>"#.to_string()
}
fn app_properties() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><Application>OOMU Artifact Builder</Application><AppVersion>1.0</AppVersion></Properties>"#.to_string()
}
fn core_properties(document: &ArtifactDocument) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dc:title>{}</dc:title><dc:subject>{}</dc:subject><dc:creator>{}</dc:creator><cp:keywords>{}</cp:keywords><dcterms:created xsi:type="dcterms:W3CDTF">1970-01-01T00:00:00Z</dcterms:created><dcterms:modified xsi:type="dcterms:W3CDTF">1970-01-01T00:00:00Z</dcterms:modified></cp:coreProperties>"#,
        xml(&document.metadata.title),
        xml(&document.metadata.subject),
        xml(&document.metadata.author),
        xml(&document.metadata.keywords.join(", "))
    )
}

fn styles_xml(document: &ArtifactDocument) -> String {
    let font = xml(&document.theme.font_family);
    let body = (document.theme.body_size_pt * 2.0).round() as u32;
    let title = (document.theme.title_size_pt * 2.0).round() as u32;
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="{font}" w:hAnsi="{font}" w:eastAsia="{font}"/><w:sz w:val="{body}"/><w:color w:val="{}"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="120" w:line="276" w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults>{}</w:styles>"#,
        document.theme.text_color,
        [
            style("Normal", "Normal", body, false, "111827", 120),
            style(
                "Title",
                "Title",
                title,
                true,
                &document.theme.heading_color,
                60
            ),
            style("Subtitle", "Subtitle", 24, false, "4B5563", 240),
            style(
                "Heading1",
                "Heading 1",
                32,
                true,
                &document.theme.heading_color,
                120
            ),
            style(
                "Heading2",
                "Heading 2",
                26,
                true,
                &document.theme.heading_color,
                80
            ),
            style("Quote", "Quote", body, false, "374151", 120),
            style("Caption", "Caption", 18, false, "4B5563", 80)
        ]
        .join("")
    )
}
fn style(id: &str, name: &str, size: u32, bold: bool, color: &str, after: u32) -> String {
    format!(
        r#"<w:style w:type="paragraph" w:styleId="{id}"><w:name w:val="{name}"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:before="80" w:after="{after}"/></w:pPr><w:rPr>{}<w:color w:val="{color}"/><w:sz w:val="{size}"/></w:rPr></w:style>"#,
        if bold { "<w:b/>" } else { "" }
    )
}
fn numbering_xml() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="0"><w:multiLevelType w:val="singleLevel"/><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="bullet"/><w:lvlText w:val="•"/><w:pPr><w:tabs><w:tab w:val="num" w:pos="720"/></w:tabs><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum><w:abstractNum w:abstractNumId="1"><w:multiLevelType w:val="singleLevel"/><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:pPr><w:tabs><w:tab w:val="num" w:pos="720"/></w:tabs><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num><w:num w:numId="2"><w:abstractNumId w:val="1"/></w:num></w:numbering>"#.to_string()
}
fn settings_xml() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:compat/><w:defaultTabStop w:val="720"/><w:decimalSymbol w:val="."/><w:listSeparator w:val=","/></w:settings>"#.to_string()
}

fn document_relationships(document: &ArtifactDocument, links: &[String]) -> String {
    let mut rels = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdStyles" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="rIdNumbering" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/><Relationship Id="rIdSettings" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings.xml"/>"#,
    );
    if document.header.is_some() {
        rels.push_str(r#"<Relationship Id="rIdHeader" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/>"#)
    }
    if document.footer.is_some() {
        rels.push_str(r#"<Relationship Id="rIdFooter" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer" Target="footer1.xml"/>"#)
    }
    for (index, url) in links.iter().enumerate() {
        rels.push_str(&format!(r#"<Relationship Id="rIdLink{index}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="{}" TargetMode="External"/>"#,xml(url)));
    }
    rels.push_str("</Relationships>");
    rels
}

fn document_xml(document: &ArtifactDocument) -> String {
    let mut body = String::new();
    body.push_str(&paragraph(&document.metadata.title, "Title", None));
    if !document.metadata.subtitle.is_empty() {
        body.push_str(&paragraph(&document.metadata.subtitle, "Subtitle", None));
    }
    let mut link_index = 0usize;
    for section in &document.sections {
        if section.page_break_before {
            body.push_str(&page_break());
        }
        body.push_str(&paragraph(&section.heading, "Heading1", None));
        for block in &section.blocks {
            match block {
                ArtifactBlock::Paragraph { text, style, .. } => body.push_str(&paragraph(
                    text,
                    match style {
                        ParagraphStyle::Body | ParagraphStyle::Lead => "Normal",
                        ParagraphStyle::Quote => "Quote",
                        ParagraphStyle::Caption => "Caption",
                    },
                    None,
                )),
                ArtifactBlock::List { ordered, items, .. } => {
                    for item in items {
                        body.push_str(&paragraph(
                            item,
                            "Normal",
                            Some(if *ordered { 2 } else { 1 }),
                        ))
                    }
                }
                ArtifactBlock::Table {
                    headers,
                    rows,
                    caption,
                    ..
                } => {
                    if !caption.is_empty() {
                        body.push_str(&paragraph(caption, "Caption", None));
                    }
                    body.push_str(&table_xml(headers, rows));
                }
                ArtifactBlock::Callout { label, text, .. } => {
                    body.push_str(&callout_xml(label, text, &document.theme.accent_color))
                }
                ArtifactBlock::Citation { label, url, .. } => {
                    body.push_str(&format!(r#"<w:p><w:hyperlink r:id="rIdLink{link_index}" w:history="1"><w:r><w:rPr><w:color w:val="2563EB"/><w:u w:val="single"/></w:rPr><w:t xml:space="preserve">{}</w:t></w:r></w:hyperlink></w:p>"#,xml(label)));
                    let _ = url;
                    link_index += 1;
                }
                ArtifactBlock::PageBreak => body.push_str(&page_break()),
            }
        }
    }
    let height = 15840u32;
    let page_width = 12240u32;
    let top = (document.page.margin_top_in * 1440.0).round() as u32;
    let right = (document.page.margin_right_in * 1440.0).round() as u32;
    let bottom = (document.page.margin_bottom_in * 1440.0).round() as u32;
    let left = (document.page.margin_left_in * 1440.0).round() as u32;
    let header_ref = if document.header.is_some() {
        r#"<w:headerReference w:type="default" r:id="rIdHeader"/>"#
    } else {
        ""
    };
    let footer_ref = if document.footer.is_some() {
        r#"<w:footerReference w:type="default" r:id="rIdFooter"/>"#
    } else {
        ""
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body>{body}<w:sectPr>{header_ref}{footer_ref}<w:pgSz w:w="{page_width}" w:h="{height}"/><w:pgMar w:top="{top}" w:right="{right}" w:bottom="{bottom}" w:left="{left}" w:header="720" w:footer="720" w:gutter="0"/><w:cols w:space="720"/><w:docGrid w:linePitch="360"/></w:sectPr></w:body></w:document>"#
    )
}
fn paragraph(text: &str, style: &str, num: Option<u32>) -> String {
    let num_xml = num
        .map(|id| format!(r#"<w:numPr><w:ilvl w:val="0"/><w:numId w:val="{id}"/></w:numPr>"#))
        .unwrap_or_default();
    format!(
        r#"<w:p><w:pPr><w:pStyle w:val="{style}"/>{num_xml}</w:pPr><w:r><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
        xml(text)
    )
}
fn page_break() -> String {
    r#"<w:p><w:r><w:br w:type="page"/></w:r></w:p>"#.to_string()
}
fn table_xml(headers: &[String], rows: &[Vec<String>]) -> String {
    let columns = headers.len() as u32;
    let total = 9240u32;
    let cell = total / columns;
    let grid = (0..columns)
        .map(|_| format!(r#"<w:gridCol w:w="{cell}"/>"#))
        .collect::<String>();
    let mut content = table_row(headers, cell, true);
    for row in rows {
        content.push_str(&table_row(row, cell, false));
    }
    format!(
        r#"<w:tbl><w:tblPr><w:tblW w:w="{total}" w:type="dxa"/><w:tblInd w:w="120" w:type="dxa"/><w:tblLayout w:type="fixed"/><w:tblBorders><w:top w:val="single" w:sz="4" w:color="D1D5DB"/><w:left w:val="single" w:sz="4" w:color="D1D5DB"/><w:bottom w:val="single" w:sz="4" w:color="D1D5DB"/><w:right w:val="single" w:sz="4" w:color="D1D5DB"/><w:insideH w:val="single" w:sz="4" w:color="E5E7EB"/><w:insideV w:val="single" w:sz="4" w:color="E5E7EB"/></w:tblBorders><w:tblCellMar><w:top w:w="100" w:type="dxa"/><w:left w:w="120" w:type="dxa"/><w:bottom w:w="100" w:type="dxa"/><w:right w:w="120" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid>{grid}</w:tblGrid>{content}</w:tbl>"#
    )
}
fn table_row(values: &[String], width: u32, header: bool) -> String {
    let cells=values.iter().map(|value|format!(r#"<w:tc><w:tcPr><w:tcW w:w="{width}" w:type="dxa"/><w:vAlign w:val="center"/>{}</w:tcPr><w:p><w:r>{}<w:t xml:space="preserve">{}</w:t></w:r></w:p></w:tc>"#,if header{r#"<w:shd w:val="clear" w:fill="E5E7EB"/>"#}else{""},if header{"<w:rPr><w:b/></w:rPr>"}else{""},xml(value))).collect::<String>();
    format!(
        r#"<w:tr>{}{cells}</w:tr>"#,
        if header {
            r#"<w:trPr><w:tblHeader/></w:trPr>"#
        } else {
            ""
        }
    )
}
fn callout_xml(label: &str, text: &str, color: &str) -> String {
    format!(
        r#"<w:tbl><w:tblPr><w:tblW w:w="9240" w:type="dxa"/><w:tblInd w:w="120" w:type="dxa"/><w:tblLayout w:type="fixed"/></w:tblPr><w:tblGrid><w:gridCol w:w="9240"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcW w:w="9240" w:type="dxa"/><w:shd w:val="clear" w:fill="EFF6FF"/><w:tcBorders><w:left w:val="single" w:sz="18" w:color="{color}"/></w:tcBorders><w:tcMar><w:top w:w="140" w:type="dxa"/><w:left w:w="180" w:type="dxa"/><w:bottom w:w="140" w:type="dxa"/><w:right w:w="180" w:type="dxa"/></w:tcMar></w:tcPr><w:p><w:r><w:rPr><w:b/></w:rPr><w:t>{}</w:t></w:r></w:p><w:p><w:r><w:t xml:space="preserve">{}</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#,
        xml(label),
        xml(text)
    )
}
fn header_footer_xml(kind: &str, text: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:{kind} xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:rPr><w:color w:val="6B7280"/><w:sz w:val="18"/></w:rPr><w:t>{}</w:t></w:r></w:p></w:{kind}>"#,
        xml(text)
    )
}

pub(crate) fn write_pdf(document: &ArtifactDocument, path: &Path) -> Result<(), String> {
    let mut pdf = Document::with_version("1.7");
    let pages_id = pdf.new_object_id();
    let font_regular=pdf.add_object(dictionary!{"Type"=>"Font","Subtype"=>"Type1","BaseFont"=>"Helvetica","Encoding"=>"WinAnsiEncoding"});
    let font_bold=pdf.add_object(dictionary!{"Type"=>"Font","Subtype"=>"Type1","BaseFont"=>"Helvetica-Bold","Encoding"=>"WinAnsiEncoding"});
    let resources =
        pdf.add_object(dictionary! {"Font"=>dictionary!{"F1"=>font_regular,"F2"=>font_bold}});
    let mut pages = Vec::new();
    let mut operations = Vec::new();
    let mut y = 742.0f32;
    let left = 72.0f32;
    let new_page = |pdf: &mut Document,
                    operations: &mut Vec<Operation>,
                    pages: &mut Vec<(u32, u16)>,
                    resources|
     -> Result<(), String> {
        if operations.is_empty() {
            return Ok(());
        }
        if let Some(header) = document.header.as_deref() {
            add_pdf_text(operations, 72.0, 772.0, 8.0, false, header);
        }
        if let Some(footer) = document.footer.as_deref() {
            add_pdf_text(operations, 72.0, 28.0, 8.0, false, footer);
        }
        add_pdf_text(
            operations,
            520.0,
            28.0,
            8.0,
            false,
            &format!("{}", pages.len() + 1),
        );
        operations.push(Operation::new("ET", vec![]));
        let content = Content {
            operations: std::mem::take(operations),
        }
        .encode()
        .map_err(|error| error.to_string())?;
        let content_id = pdf.add_object(Stream::new(dictionary! {}, content));
        let page_id=pdf.add_object(dictionary!{"Type"=>"Page","Parent"=>pages_id,"MediaBox"=>vec![0.into(),0.into(),612.into(),792.into()],"Resources"=>resources,"Contents"=>content_id});
        pages.push(page_id);
        Ok(())
    };
    operations.push(Operation::new("BT", vec![]));
    add_pdf_text(
        &mut operations,
        left,
        y,
        26.0,
        true,
        &document.metadata.title,
    );
    y -= 38.0;
    if !document.metadata.subtitle.is_empty() {
        add_pdf_text(
            &mut operations,
            left,
            y,
            12.0,
            false,
            &document.metadata.subtitle,
        );
        y -= 26.0;
    }
    for section in &document.sections {
        if section.page_break_before && !pages.is_empty() {
            new_page(&mut pdf, &mut operations, &mut pages, resources)?;
            operations.push(Operation::new("BT", vec![]));
            y = 742.0;
        }
        ensure_pdf_space(
            &mut pdf,
            &mut operations,
            &mut pages,
            resources,
            &mut y,
            70.0,
            &new_page,
        )?;
        add_pdf_text(&mut operations, left, y, 17.0, true, &section.heading);
        y -= 28.0;
        for block in &section.blocks {
            match block {
                ArtifactBlock::Paragraph { text, .. } => {
                    for line in wrap(text, 86) {
                        ensure_pdf_space(
                            &mut pdf,
                            &mut operations,
                            &mut pages,
                            resources,
                            &mut y,
                            18.0,
                            &new_page,
                        )?;
                        add_pdf_text(&mut operations, left, y, 10.5, false, &line);
                        y -= 15.0;
                    }
                }
                ArtifactBlock::List { ordered, items, .. } => {
                    for (i, item) in items.iter().enumerate() {
                        for (j, line) in wrap(item, 80).into_iter().enumerate() {
                            ensure_pdf_space(
                                &mut pdf,
                                &mut operations,
                                &mut pages,
                                resources,
                                &mut y,
                                18.0,
                                &new_page,
                            )?;
                            let prefix = if j == 0 {
                                if *ordered {
                                    format!("{}. ", i + 1)
                                } else {
                                    "- ".to_string()
                                }
                            } else {
                                "  ".to_string()
                            };
                            add_pdf_text(
                                &mut operations,
                                left + 12.0,
                                y,
                                10.5,
                                false,
                                &format!("{prefix}{line}"),
                            );
                            y -= 15.0;
                        }
                    }
                }
                ArtifactBlock::Table {
                    headers,
                    rows,
                    caption,
                    ..
                } => {
                    if !caption.is_empty() {
                        add_pdf_text(&mut operations, left, y, 9.0, true, caption);
                        y -= 15.0;
                    }
                    for (index, row) in std::iter::once(headers).chain(rows.iter()).enumerate() {
                        ensure_pdf_space(
                            &mut pdf,
                            &mut operations,
                            &mut pages,
                            resources,
                            &mut y,
                            20.0,
                            &new_page,
                        )?;
                        let text = row
                            .iter()
                            .map(|cell| truncate(cell, 24))
                            .collect::<Vec<_>>()
                            .join(" | ");
                        add_pdf_text(
                            &mut operations,
                            left,
                            y,
                            if index == 0 { 9.5 } else { 9.0 },
                            index == 0,
                            &text,
                        );
                        y -= 16.0;
                    }
                    y -= 5.0;
                }
                ArtifactBlock::Callout { label, text, .. } => {
                    ensure_pdf_space(
                        &mut pdf,
                        &mut operations,
                        &mut pages,
                        resources,
                        &mut y,
                        45.0,
                        &new_page,
                    )?;
                    add_pdf_text(&mut operations, left + 10.0, y, 10.0, true, label);
                    y -= 15.0;
                    for line in wrap(text, 80) {
                        add_pdf_text(&mut operations, left + 10.0, y, 9.5, false, &line);
                        y -= 14.0;
                    }
                    y -= 8.0;
                }
                ArtifactBlock::Citation { label, url, .. } => {
                    ensure_pdf_space(
                        &mut pdf,
                        &mut operations,
                        &mut pages,
                        resources,
                        &mut y,
                        30.0,
                        &new_page,
                    )?;
                    add_pdf_text(
                        &mut operations,
                        left,
                        y,
                        9.5,
                        false,
                        &format!("{label} - {url}"),
                    );
                    y -= 16.0;
                }
                ArtifactBlock::PageBreak => {
                    new_page(&mut pdf, &mut operations, &mut pages, resources)?;
                    operations.push(Operation::new("BT", vec![]));
                    y = 742.0;
                }
            }
            y -= 10.0;
        }
    }
    new_page(&mut pdf, &mut operations, &mut pages, resources)?;
    let urls = document
        .sections
        .iter()
        .flat_map(|section| section.blocks.iter())
        .filter_map(|block| {
            if let ArtifactBlock::Citation { url, .. } = block {
                Some(url)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let mut annotation_ids = Vec::new();
    for (index, url) in urls.into_iter().enumerate() {
        let action =
            pdf.add_object(dictionary! {"S"=>"URI","URI"=>Object::string_literal(url.as_bytes())});
        let y = 70 + (index as i64 % 20) * 14;
        annotation_ids.push(pdf.add_object(dictionary!{"Type"=>"Annot","Subtype"=>"Link","Rect"=>vec![72.into(),y.into(),540.into(),(y+12).into()],"Border"=>vec![0.into(),0.into(),0.into()],"A"=>action}));
    }
    if let Some(first_page) = pages.first().copied() {
        if !annotation_ids.is_empty() {
            pdf.get_object_mut(first_page)
                .map_err(|error| error.to_string())?
                .as_dict_mut()
                .map_err(|error| error.to_string())?
                .set(
                    "Annots",
                    annotation_ids
                        .into_iter()
                        .map(Object::Reference)
                        .collect::<Vec<_>>(),
                );
        }
    }
    pdf.objects.insert(pages_id,Object::Dictionary(dictionary!{"Type"=>"Pages","Kids"=>pages.iter().map(|id|Object::Reference(*id)).collect::<Vec<_>>(),"Count"=>pages.len() as i64}));
    let catalog = pdf.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages_id});
    let info=pdf.add_object(dictionary!{"Title"=>Object::string_literal(document.metadata.title.as_bytes()),"Author"=>Object::string_literal(document.metadata.author.as_bytes()),"Subject"=>Object::string_literal(document.metadata.subject.as_bytes()),"Producer"=>ARTIFACT_BUILDER_IDENTITY});
    pdf.trailer.set("Root", catalog);
    pdf.trailer.set("Info", info);
    pdf.compress();
    pdf.save(path).map_err(|error| error.to_string())?;
    Ok(())
}
fn ensure_pdf_space<F>(
    pdf: &mut Document,
    operations: &mut Vec<Operation>,
    pages: &mut Vec<(u32, u16)>,
    resources: (u32, u16),
    y: &mut f32,
    needed: f32,
    new_page: &F,
) -> Result<(), String>
where
    F: Fn(
        &mut Document,
        &mut Vec<Operation>,
        &mut Vec<(u32, u16)>,
        (u32, u16),
    ) -> Result<(), String>,
{
    if *y - needed < 62.0 {
        new_page(pdf, operations, pages, resources)?;
        operations.push(Operation::new("BT", vec![]));
        *y = 742.0;
    }
    Ok(())
}
fn add_pdf_text(ops: &mut Vec<Operation>, x: f32, y: f32, size: f32, bold: bool, text: &str) {
    ops.push(Operation::new(
        "Tf",
        vec![
            Object::Name(if bold { b"F2".to_vec() } else { b"F1".to_vec() }),
            size.into(),
        ],
    ));
    ops.push(Operation::new(
        "Tm",
        vec![1.into(), 0.into(), 0.into(), 1.into(), x.into(), y.into()],
    ));
    ops.push(Operation::new(
        "Tj",
        vec![Object::string_literal(pdf_text(text))],
    ));
    // Delimit each positioned text run so independent PDF extractors preserve
    // the same word and line boundaries that the rendered page presents.
    ops.push(Operation::new("ET", vec![]));
    ops.push(Operation::new("BT", vec![]));
}
fn pdf_text(value: &str) -> Vec<u8> {
    value
        .chars()
        .map(|character| {
            if character.is_ascii() {
                character as u8
            } else {
                b'?'
            }
        })
        .collect()
}
fn wrap(value: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in value.split_whitespace() {
        if !current.is_empty() && current.len() + 1 + word.len() > width {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}
fn truncate(value: &str, max: usize) -> String {
    let mut chars = value.chars();
    let out = chars.by_ref().take(max).collect::<String>();
    if chars.next().is_some() {
        format!("{out}...")
    } else {
        out
    }
}

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn xml_escaping_never_allows_render_code() {
        assert_eq!(xml("<script>&\""), "&lt;script&gt;&amp;&quot;");
    }
    #[test]
    fn wrapping_is_bounded() {
        assert!(wrap(&"word ".repeat(100), 20)
            .iter()
            .all(|line| line.len() <= 20));
    }
}
