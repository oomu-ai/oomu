use super::{ArtifactDocument, ArtifactVerification, ARTIFACT_RENDERER_IDENTITY};
use crc32fast::Hasher;
use lopdf::{Document, Object};
use serde::Deserialize;
use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const MAX_DOCX_BYTES: usize = 32 * 1024 * 1024;
const RENDER_TIMEOUT: Duration = Duration::from_secs(30);

pub(super) fn verify_all(
    document: &ArtifactDocument,
    docx: &Path,
    pdf: &Path,
    render_dir: &Path,
) -> Result<(ArtifactVerification, Vec<PathBuf>), String> {
    verify_docx(document, docx)?;
    let page_count = verify_pdf(document, pdf)?;
    let (renderer, pages, warnings) = render_pdf(pdf, render_dir)?;
    if renderer != ARTIFACT_RENDERER_IDENTITY || pages.len() != page_count {
        return Err(
            "Artifact PDF rendering did not cover every structurally verified page.".to_string(),
        );
    }
    for page in &pages {
        verify_page_image(page)?;
    }
    Ok((
        ArtifactVerification {
            structurally_verified_docx: true,
            structurally_verified_pdf: true,
            visually_verified_pdf: true,
            page_count,
            warnings,
            renderer_probe: renderer,
        },
        pages,
    ))
}

fn verify_docx(document: &ArtifactDocument, path: &Path) -> Result<(), String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("DOCX verification could not read output: {error}"))?;
    if bytes.is_empty() || bytes.len() > MAX_DOCX_BYTES {
        return Err("DOCX output failed its size limit.".to_string());
    }
    let entries = parse_store_zip(&bytes)?;
    for required in [
        "[Content_Types].xml",
        "_rels/.rels",
        "docProps/core.xml",
        "word/document.xml",
        "word/styles.xml",
        "word/numbering.xml",
        "word/_rels/document.xml.rels",
    ] {
        if !entries.contains_key(required) {
            return Err(format!("DOCX package is missing required part {required}."));
        }
    }
    if entries.keys().any(|name| {
        name.ends_with("vbaProject.bin")
            || name.ends_with("attachedTemplate.bin")
            || name.contains("embeddings/")
    }) {
        return Err(
            "DOCX package contains macros, templates, or embedded executable content.".to_string(),
        );
    }
    let content = xml_entry(&entries, "[Content_Types].xml")?;
    if content.contains("macroEnabled") || content.contains("vnd.ms-office.vbaProject") {
        return Err("DOCX content types declare macro content.".to_string());
    }
    let relationships = xml_entry(&entries, "word/_rels/document.xml.rels")?;
    for fragment in relationships.split("<Relationship").skip(1) {
        if fragment.contains("TargetMode=\"External\"") && !fragment.contains("/hyperlink\"") {
            return Err(
                "DOCX contains an external relationship outside a declared hyperlink.".to_string(),
            );
        }
        if fragment.contains("attachedTemplate") {
            return Err("DOCX contains an external template relationship.".to_string());
        }
    }
    let main = xml_entry(&entries, "word/document.xml")?;
    if !main.contains("<w:document")
        || !main.contains("<w:sectPr")
        || !main.contains(&xml_escape(&document.metadata.title))
    {
        return Err("DOCX document XML is incomplete or missing its title.".to_string());
    }
    if main.contains("<script") || main.contains("javascript:") {
        return Err("DOCX contains prohibited executable markup.".to_string());
    }
    Ok(())
}

pub(super) fn verify_pdf(document: &ArtifactDocument, path: &Path) -> Result<usize, String> {
    let pdf =
        Document::load(path).map_err(|error| format!("PDF structural parse failed: {error}"))?;
    if pdf.is_encrypted() {
        return Err("PDF output is unexpectedly encrypted.".to_string());
    }
    let pages = pdf.get_pages();
    if pages.is_empty() || pages.len() > 128 {
        return Err("PDF page count is outside supported bounds.".to_string());
    }
    let expected_links = document
        .sections
        .iter()
        .flat_map(|section| section.blocks.iter())
        .filter_map(|block| match block {
            super::ArtifactBlock::Citation { url, .. } => Some(url.clone()),
            _ => None,
        })
        .fold(HashMap::<String, usize>::new(), |mut links, url| {
            *links.entry(url).or_default() += 1;
            links
        });
    let mut text = String::new();
    for (number, id) in &pages {
        let page = pdf
            .get_object(*id)
            .map_err(|error| error.to_string())?
            .as_dict()
            .map_err(|error| error.to_string())?;
        let contents = page
            .get(b"Contents")
            .map_err(|_| format!("PDF page {number} has no content stream."))?;
        let ids = match contents {
            Object::Reference(id) => vec![*id],
            Object::Array(values) => values
                .iter()
                .filter_map(|value| value.as_reference().ok())
                .collect(),
            _ => Vec::new(),
        };
        if ids.is_empty() {
            return Err(format!("PDF page {number} has no usable content stream."));
        }
        for id in ids {
            let stream = pdf
                .get_object(id)
                .map_err(|error| error.to_string())?
                .as_stream()
                .map_err(|error| error.to_string())?;
            if stream.content.is_empty() {
                return Err(format!("PDF page {number} has an empty content stream."));
            }
        }
        text.push_str(
            &pdf.extract_text(&[*number])
                .map_err(|_| format!("PDF page {number} text extraction failed."))?,
        );
    }
    verify_pdf_text_content(document, &text)?;
    let mut verified_links = HashMap::<String, usize>::new();
    for page_id in pages.values() {
        let page = pdf
            .get_object(*page_id)
            .map_err(|error| error.to_string())?
            .as_dict()
            .map_err(|error| error.to_string())?;
        let Some(annotations) = page
            .get(b"Annots")
            .ok()
            .and_then(|value| resolved_pdf_object(&pdf, value))
            .and_then(|value| value.as_array().ok())
        else {
            continue;
        };
        for annotation in annotations {
            let Some(annotation) =
                resolved_pdf_object(&pdf, annotation).and_then(|value| value.as_dict().ok())
            else {
                continue;
            };
            let is_link = annotation
                .get(b"Subtype")
                .ok()
                .and_then(|value| resolved_pdf_object(&pdf, value))
                .and_then(|value| value.as_name().ok())
                == Some(b"Link");
            if !is_link {
                continue;
            }
            let Some(action) = annotation
                .get(b"A")
                .ok()
                .and_then(|value| resolved_pdf_object(&pdf, value))
                .and_then(|value| value.as_dict().ok())
            else {
                continue;
            };
            let is_uri_action = action
                .get(b"S")
                .ok()
                .and_then(|value| resolved_pdf_object(&pdf, value))
                .and_then(|value| value.as_name().ok())
                == Some(b"URI");
            if !is_uri_action {
                continue;
            }
            let Some(uri) = action
                .get(b"URI")
                .ok()
                .and_then(|value| resolved_pdf_object(&pdf, value))
                .and_then(pdf_string)
            else {
                continue;
            };
            *verified_links.entry(uri.to_string()).or_default() += 1;
        }
    }
    if verified_links != expected_links {
        return Err("PDF output citation links do not match the expected URLs.".to_string());
    }
    let info = pdf
        .trailer
        .get(b"Info")
        .map_err(|_| "PDF metadata dictionary is missing.".to_string())?
        .as_reference()
        .map_err(|_| "PDF metadata reference is invalid.".to_string())?;
    let info = pdf
        .get_object(info)
        .map_err(|error| error.to_string())?
        .as_dict()
        .map_err(|error| error.to_string())?;
    if info.get(b"Title").is_err() || info.get(b"Producer").is_err() {
        return Err("PDF metadata is incomplete.".to_string());
    }
    Ok(pages.len())
}

fn verify_pdf_text_content(document: &ArtifactDocument, extracted: &str) -> Result<(), String> {
    let title_words = document
        .metadata
        .title
        .split_whitespace()
        .filter(|word| word.is_ascii())
        .collect::<Vec<_>>();
    if title_words.iter().any(|word| !extracted.contains(word)) {
        return Err("PDF output does not contain the document title text.".to_string());
    }
    verify_expected_pdf_content(document, extracted)
}

fn verify_expected_pdf_content(document: &ArtifactDocument, extracted: &str) -> Result<(), String> {
    let mut expected = Vec::<String>::new();
    append_content_tokens(&mut expected, &document.metadata.title);
    append_content_tokens(&mut expected, &document.metadata.subtitle);
    for section in &document.sections {
        if !section
            .heading
            .eq_ignore_ascii_case(&document.metadata.title)
        {
            append_content_tokens(&mut expected, &section.heading);
        }
        for block in &section.blocks {
            match block {
                super::ArtifactBlock::Paragraph { text, .. } => {
                    append_content_tokens(&mut expected, text)
                }
                super::ArtifactBlock::List { items, .. } => {
                    for item in items {
                        append_content_tokens(&mut expected, item);
                    }
                }
                super::ArtifactBlock::Table {
                    headers,
                    rows,
                    caption,
                    ..
                } => {
                    append_content_tokens(&mut expected, caption);
                    for value in headers.iter().chain(rows.iter().flatten()) {
                        append_content_tokens(&mut expected, value);
                    }
                }
                super::ArtifactBlock::Callout { label, text, .. } => {
                    append_content_tokens(&mut expected, label);
                    append_content_tokens(&mut expected, text);
                }
                super::ArtifactBlock::Citation { label, url, .. } => {
                    append_content_tokens(&mut expected, label);
                    append_content_tokens(&mut expected, url);
                }
                super::ArtifactBlock::PageBreak => {}
            }
        }
    }
    let actual = content_tokens(extracted);
    let mut actual_index = 0usize;
    for token in expected {
        let Some(offset) = actual[actual_index..]
            .iter()
            .position(|candidate| candidate == &token)
        else {
            return Err("PDF output omitted expected document content.".to_string());
        };
        actual_index += offset + 1;
    }
    Ok(())
}

fn append_content_tokens(tokens: &mut Vec<String>, value: &str) {
    tokens.extend(content_tokens(value));
}

fn content_tokens(value: &str) -> Vec<String> {
    value
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_lowercase())
        .collect()
}

fn resolved_pdf_object<'a>(pdf: &'a Document, value: &'a Object) -> Option<&'a Object> {
    let mut current = value;
    for _ in 0..16 {
        match current {
            Object::Reference(id) => current = pdf.get_object(*id).ok()?,
            _ => return Some(current),
        }
    }
    None
}

fn pdf_string(value: &Object) -> Option<&str> {
    match value {
        Object::String(bytes, _) => std::str::from_utf8(bytes).ok(),
        _ => None,
    }
}

#[derive(Debug, Deserialize)]
struct RenderOutput {
    backend: String,
    page_count: usize,
    page_files: Vec<String>,
    warnings: Vec<String>,
}
fn render_pdf(pdf: &Path, output: &Path) -> Result<(String, Vec<PathBuf>, Vec<String>), String> {
    fs::create_dir_all(output)
        .map_err(|error| format!("Unable to create private PDF render directory: {error}"))?;
    let helper =
        resolve_renderer().ok_or_else(|| "Packaged PDF renderer is unavailable.".to_string())?;
    let probe = Command::new(&helper)
        .arg("--probe-pdf-renderer")
        .env_clear()
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|error| format!("PDF renderer startup probe failed: {error}"))?;
    if !probe.status.success()
        || !String::from_utf8_lossy(&probe.stdout).contains("apple-pdfkit-v1")
    {
        return Err("Packaged PDF renderer startup probe did not verify its identity.".to_string());
    }
    let mut child = Command::new(&helper)
        .arg("--render-pdf")
        .arg(pdf)
        .arg(output)
        .env_clear()
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("PDF renderer failed to start: {error}"))?;
    let started = Instant::now();
    loop {
        match child.try_wait().map_err(|error| error.to_string())? {
            Some(_) => break,
            None if started.elapsed() < RENDER_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(10))
            }
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("PDF renderer exceeded its wall-time limit.".to_string());
            }
        }
    }
    let result = child
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    let parsed: RenderOutput = serde_json::from_slice(&result.stdout)
        .map_err(|error| format!("PDF renderer returned invalid protocol data: {error}"))?;
    if !result.status.success()
        || parsed.backend != ARTIFACT_RENDERER_IDENTITY
        || parsed.page_count == 0
        || parsed.page_count != parsed.page_files.len()
    {
        return Err("PDF renderer did not produce every required page image.".to_string());
    }
    let root = fs::canonicalize(output).map_err(|error| error.to_string())?;
    let mut pages = Vec::new();
    for raw in parsed.page_files {
        let path = PathBuf::from(raw);
        let metadata =
            fs::symlink_metadata(&path).map_err(|_| "Rendered PDF page is missing.".to_string())?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() == 0
            || metadata.len() > 16 * 1024 * 1024
        {
            return Err("Rendered PDF page failed file validation.".to_string());
        }
        let canonical = fs::canonicalize(path).map_err(|error| error.to_string())?;
        if !canonical.starts_with(&root) {
            return Err("Rendered PDF page escaped private staging.".to_string());
        }
        pages.push(canonical);
    }
    Ok((
        ARTIFACT_RENDERER_IDENTITY.to_string(),
        pages,
        parsed.warnings,
    ))
}

fn verify_page_image(path: &Path) -> Result<(), String> {
    let image = image::ImageReader::open(path)
        .map_err(|error| error.to_string())?
        .with_guessed_format()
        .map_err(|error| error.to_string())?
        .decode()
        .map_err(|error| format!("Rendered PDF page is invalid: {error}"))?
        .to_luma8();
    if image.width() < 300 || image.height() < 300 || image.width() > 4000 || image.height() > 4000
    {
        return Err("Rendered PDF page dimensions are invalid.".to_string());
    }
    let mut min = 255u8;
    let mut max = 0u8;
    let mut dark = 0usize;
    for pixel in image.pixels() {
        let value = pixel[0];
        min = min.min(value);
        max = max.max(value);
        if value < 245 {
            dark += 1;
        }
    }
    if max.saturating_sub(min) < 24 || dark < 50 {
        return Err("Rendered PDF page appears blank or unreadable.".to_string());
    }
    Ok(())
}

fn resolve_renderer() -> Option<PathBuf> {
    let filename = if cfg!(windows) {
        "oomu-artifact-pdf-helper.exe"
    } else {
        "oomu-artifact-pdf-helper"
    };
    let sibling = env::current_exe().ok()?.parent()?.join(filename);
    if sibling.is_file() {
        return Some(sibling);
    }
    #[cfg(debug_assertions)]
    {
        let triple = std::process::Command::new("rustc")
            .args(["--print", "host-tuple"])
            .output()
            .ok()
            .and_then(|value| String::from_utf8(value.stdout).ok())
            .map(|value| value.trim().to_string())?;
        let candidate = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("binaries")
            .join(format!("oomu-artifact-pdf-helper-{triple}"));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}
pub(super) fn probe_renderer() -> Result<(), String> {
    let helper =
        resolve_renderer().ok_or_else(|| "Packaged PDF renderer is unavailable.".to_string())?;
    let output = Command::new(helper)
        .arg("--probe-pdf-renderer")
        .env_clear()
        .output()
        .map_err(|error| format!("PDF renderer startup probe failed: {error}"))?;
    if !output.status.success()
        || !String::from_utf8_lossy(&output.stdout).contains("apple-pdfkit-v1")
    {
        return Err(
            "Packaged PDF renderer startup probe returned an invalid identity.".to_string(),
        );
    }
    Ok(())
}

fn parse_store_zip(bytes: &[u8]) -> Result<HashMap<String, Vec<u8>>, String> {
    let mut cursor = 0usize;
    let mut entries = HashMap::new();
    while cursor + 4 <= bytes.len() {
        let signature = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap());
        if signature == 0x02014b50 || signature == 0x06054b50 {
            break;
        }
        if signature != 0x04034b50 || cursor + 30 > bytes.len() {
            return Err("DOCX ZIP local header is invalid.".to_string());
        }
        let flags = u16::from_le_bytes(bytes[cursor + 6..cursor + 8].try_into().unwrap());
        let method = u16::from_le_bytes(bytes[cursor + 8..cursor + 10].try_into().unwrap());
        let crc = u32::from_le_bytes(bytes[cursor + 14..cursor + 18].try_into().unwrap());
        let compressed =
            u32::from_le_bytes(bytes[cursor + 18..cursor + 22].try_into().unwrap()) as usize;
        let uncompressed =
            u32::from_le_bytes(bytes[cursor + 22..cursor + 26].try_into().unwrap()) as usize;
        let name_len =
            u16::from_le_bytes(bytes[cursor + 26..cursor + 28].try_into().unwrap()) as usize;
        let extra_len =
            u16::from_le_bytes(bytes[cursor + 28..cursor + 30].try_into().unwrap()) as usize;
        if flags & 0x0001 != 0 || method != 0 || compressed != uncompressed {
            return Err("DOCX ZIP uses unsupported encryption or compression.".to_string());
        }
        let name_start = cursor + 30;
        let data_start = name_start
            .checked_add(name_len + extra_len)
            .ok_or_else(|| "DOCX ZIP header overflow.".to_string())?;
        let data_end = data_start
            .checked_add(compressed)
            .ok_or_else(|| "DOCX ZIP data overflow.".to_string())?;
        if data_end > bytes.len() {
            return Err("DOCX ZIP entry is truncated.".to_string());
        }
        let name = std::str::from_utf8(&bytes[name_start..name_start + name_len])
            .map_err(|_| "DOCX ZIP filename is invalid UTF-8.".to_string())?
            .to_string();
        if name.starts_with('/') || name.contains("..") || entries.contains_key(&name) {
            return Err("DOCX ZIP entry path is unsafe or duplicated.".to_string());
        }
        let data = bytes[data_start..data_end].to_vec();
        let mut hasher = Hasher::new();
        hasher.update(&data);
        if hasher.finalize() != crc {
            return Err("DOCX ZIP entry CRC verification failed.".to_string());
        }
        entries.insert(name, data);
        cursor = data_end;
    }
    if entries.is_empty() {
        return Err("DOCX ZIP contains no package parts.".to_string());
    }
    Ok(entries)
}
fn xml_entry<'a>(entries: &'a HashMap<String, Vec<u8>>, name: &str) -> Result<&'a str, String> {
    std::str::from_utf8(
        entries
            .get(name)
            .ok_or_else(|| format!("DOCX part {name} is missing."))?,
    )
    .map_err(|_| format!("DOCX part {name} is not UTF-8 XML."))
}
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifacts::{
        ArtifactBlock, ArtifactMetadata, ArtifactSection, PageControls, ParagraphStyle,
        ThemeTokens, ARTIFACT_DOCUMENT_SCHEMA_VERSION,
    };
    fn document() -> ArtifactDocument {
        ArtifactDocument{schema_version:ARTIFACT_DOCUMENT_SCHEMA_VERSION,metadata:ArtifactMetadata{title:"Verified Report".into(),subtitle:"Editable and rendered".into(),author:"OOMU".into(),subject:"Pipeline test".into(),keywords:vec!["verification".into()],language:"en".into()},theme:ThemeTokens::default(),page:PageControls::default(),header:Some("Verified Report".into()),footer:Some("OOMU".into()),sections:vec![ArtifactSection{heading:"Findings".into(),page_break_before:false,blocks:vec![ArtifactBlock::Paragraph{text:"A real editable paragraph with enough visible content to verify the output page.".into(),style:ParagraphStyle::Body,factual:false,sources:vec![]},ArtifactBlock::Table{headers:vec!["Check".into(),"Result".into()],rows:vec![vec!["Structure".into(),"Passed".into()]],caption:"Verification matrix".into(),factual:false,sources:vec![]},ArtifactBlock::Citation{label:"Example source".into(),url:"https://example.com/source".into(),source_ref:"source-1".into(),evidence_ref:"evidence-1".into()}]}]}
    }
    #[test]
    fn truncated_zip_is_rejected() {
        assert!(parse_store_zip(b"PK\x03\x04").is_err());
    }
    #[test]
    fn real_docx_and_pdf_pass_independent_structural_validation() {
        let root = std::env::temp_dir().join(format!(
            "oomu-artifact-verify-{}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        fs::create_dir_all(&root).unwrap();
        let document = document();
        let docx = root.join("artifact.docx");
        let pdf = root.join("artifact.pdf");
        super::super::helper::write_docx(&document, &docx).unwrap();
        super::super::helper::write_pdf(&document, &pdf).unwrap();
        verify_docx(&document, &docx).unwrap();
        assert!(verify_pdf(&document, &pdf).unwrap() > 0);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn pdf_missing_the_expected_tail_fails_closed() {
        let root = std::env::temp_dir().join(format!(
            "oomu-artifact-missing-tail-{}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        fs::create_dir_all(&root).unwrap();
        let mut expected = document();
        if let ArtifactBlock::Paragraph { text, .. } = &mut expected.sections[0].blocks[0] {
            text.push_str(" Required final evidence marker alpha omega.");
        }
        let mut clipped = expected.clone();
        if let ArtifactBlock::Paragraph { text, .. } = &mut clipped.sections[0].blocks[0] {
            *text = text
                .split(" Required final evidence marker")
                .next()
                .unwrap()
                .to_string();
        }
        let pdf = root.join("artifact.pdf");
        super::super::helper::write_pdf(&clipped, &pdf).unwrap();

        assert_eq!(
            verify_pdf(&expected, &pdf).unwrap_err(),
            "PDF output omitted expected document content."
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn core_graphics_indirect_citation_objects_verify_exact_urls() {
        let root = std::env::temp_dir().join(format!(
            "oomu-artifact-core-graphics-links-{}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        fs::create_dir_all(&root).unwrap();
        let document = document();
        let source = root.join("source.pdf");
        let indirect = root.join("indirect.pdf");
        let wrong = root.join("wrong.pdf");
        super::super::helper::write_pdf(&document, &source).unwrap();

        rewrite_citation_objects_as_indirect(&source, &indirect, None);
        assert!(verify_pdf(&document, &indirect).unwrap() > 0);

        rewrite_citation_objects_as_indirect(
            &source,
            &wrong,
            Some("https://example.com/a-different-source"),
        );
        assert_eq!(
            verify_pdf(&document, &wrong).unwrap_err(),
            "PDF output citation links do not match the expected URLs."
        );
        let _ = fs::remove_dir_all(root);
    }

    fn rewrite_citation_objects_as_indirect(
        source: &Path,
        destination: &Path,
        replacement_url: Option<&str>,
    ) {
        let mut pdf = Document::load(source).unwrap();
        let page_id = *pdf.get_pages().values().next().unwrap();
        let annotations = pdf
            .get_object(page_id)
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"Annots")
            .unwrap()
            .as_array()
            .unwrap()
            .clone();
        for annotation in &annotations {
            let annotation_id = annotation.as_reference().unwrap();
            let action_id = pdf
                .get_object(annotation_id)
                .unwrap()
                .as_dict()
                .unwrap()
                .get(b"A")
                .unwrap()
                .as_reference()
                .unwrap();
            let uri = replacement_url
                .map(|value| Object::string_literal(value.as_bytes()))
                .unwrap_or_else(|| {
                    pdf.get_object(action_id)
                        .unwrap()
                        .as_dict()
                        .unwrap()
                        .get(b"URI")
                        .unwrap()
                        .clone()
                });
            let uri_id = pdf.add_object(uri);
            pdf.get_object_mut(action_id)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("URI", uri_id);
        }
        let annotations_id = pdf.add_object(Object::Array(annotations));
        pdf.get_object_mut(page_id)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Annots", annotations_id);
        pdf.save(destination).unwrap();
    }
}
