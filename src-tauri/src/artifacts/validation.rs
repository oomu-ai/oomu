use super::*;

const MAX_DOCUMENT_JSON_BYTES: usize = 1024 * 1024;
const MAX_TEXT_CHARS: usize = 240_000;
const MAX_SECTIONS: usize = 100;
const MAX_BLOCKS: usize = 1_000;

pub(super) fn validate(document: &ArtifactDocument) -> Result<(), String> {
    if document.schema_version != ARTIFACT_DOCUMENT_SCHEMA_VERSION {
        return Err("Unsupported ArtifactDocument schema version.".to_string());
    }
    clean(&document.metadata.title, 1, 240, "title")?;
    clean(&document.metadata.subtitle, 0, 500, "subtitle")?;
    clean(&document.metadata.author, 0, 160, "author")?;
    if document.metadata.keywords.len() > 24 {
        return Err("Artifact metadata has too many keywords.".to_string());
    }
    validate_theme(&document.theme)?;
    validate_page(&document.page)?;
    if document.sections.is_empty() || document.sections.len() > MAX_SECTIONS {
        return Err("Artifact requires 1 to 100 sections.".to_string());
    }
    let encoded = serde_json::to_vec(document).map_err(|error| error.to_string())?;
    if encoded.len() > MAX_DOCUMENT_JSON_BYTES {
        return Err("Artifact document exceeds the 1 MB IR limit.".to_string());
    }
    let mut text_chars = 0usize;
    let mut blocks = 0usize;
    for section in &document.sections {
        clean(&section.heading, 1, 240, "section heading")?;
        if section.blocks.is_empty() {
            return Err("Artifact sections cannot be empty.".to_string());
        }
        for block in &section.blocks {
            blocks += 1;
            if blocks > MAX_BLOCKS {
                return Err("Artifact exceeds the block limit.".to_string());
            }
            validate_block(block, &mut text_chars)?;
        }
    }
    if text_chars > MAX_TEXT_CHARS {
        return Err("Artifact text exceeds the 240,000 character limit.".to_string());
    }
    Ok(())
}

fn validate_block(block: &ArtifactBlock, text_chars: &mut usize) -> Result<(), String> {
    if block.factual()
        && block.sources().is_empty()
        && !matches!(block, ArtifactBlock::Citation { .. })
    {
        return Err(
            "Every factual artifact block requires source and evidence references.".to_string(),
        );
    }
    for source in block.sources() {
        validate_source(source)?;
    }
    match block {
        ArtifactBlock::Paragraph { text, .. } => add_text(text, 1, 20_000, text_chars, "paragraph"),
        ArtifactBlock::List { items, .. } => {
            if items.is_empty() || items.len() > 100 {
                return Err("Artifact list size is invalid.".to_string());
            }
            for item in items {
                add_text(item, 1, 2_000, text_chars, "list item")?;
            }
            Ok(())
        }
        ArtifactBlock::Table {
            headers,
            rows,
            caption,
            ..
        } => {
            if headers.is_empty()
                || headers.len() > 12
                || rows.is_empty()
                || rows.len() > 30
                || rows.iter().any(|row| row.len() != headers.len())
            {
                return Err("Artifact table dimensions are invalid.".to_string());
            }
            clean(caption, 0, 500, "table caption")?;
            for value in headers.iter().chain(rows.iter().flatten()) {
                add_text(value, 0, 2_000, text_chars, "table cell")?;
            }
            Ok(())
        }
        ArtifactBlock::Callout { label, text, .. } => {
            add_text(label, 1, 120, text_chars, "callout label")?;
            add_text(text, 1, 4_000, text_chars, "callout text")
        }
        ArtifactBlock::Citation {
            label,
            url,
            source_ref,
            evidence_ref,
        } => {
            add_text(label, 1, 500, text_chars, "citation label")?;
            validate_url(url)?;
            clean(source_ref, 1, 256, "citation source")?;
            clean(evidence_ref, 1, 256, "citation evidence")
        }
        ArtifactBlock::PageBreak => Ok(()),
    }
}

fn validate_source(source: &ArtifactSourceReference) -> Result<(), String> {
    clean(&source.source_ref, 1, 256, "source reference")?;
    clean(&source.evidence_ref, 1, 256, "evidence reference")?;
    if let Some(url) = source.url.as_deref() {
        validate_url(url)?;
    }
    Ok(())
}
fn validate_url(raw: &str) -> Result<(), String> {
    let url = url::Url::parse(raw).map_err(|_| "Artifact URL is invalid.".to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Artifact URL must be a credential-free HTTP(S) URL.".to_string());
    }
    Ok(())
}
fn validate_theme(theme: &ThemeTokens) -> Result<(), String> {
    clean(&theme.font_family, 1, 80, "font family")?;
    if !(8.0..=18.0).contains(&theme.body_size_pt) || !(18.0..=42.0).contains(&theme.title_size_pt)
    {
        return Err("Artifact typography tokens are outside supported bounds.".to_string());
    }
    for color in [
        &theme.heading_color,
        &theme.accent_color,
        &theme.text_color,
        &theme.background_color,
    ] {
        if color.len() != 6 || !color.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("Artifact theme colors must be six-digit hex values.".to_string());
        }
    }
    Ok(())
}
fn validate_page(page: &PageControls) -> Result<(), String> {
    if page.size != "letter" || page.orientation != "portrait" {
        return Err("Artifact V1 supports US Letter portrait only.".to_string());
    }
    for margin in [
        page.margin_top_in,
        page.margin_right_in,
        page.margin_bottom_in,
        page.margin_left_in,
    ] {
        if !margin.is_finite() || !(0.5..=2.0).contains(&margin) {
            return Err("Artifact page margins must be between 0.5 and 2 inches.".to_string());
        }
    }
    Ok(())
}
fn add_text(
    value: &str,
    min: usize,
    max: usize,
    total: &mut usize,
    label: &str,
) -> Result<(), String> {
    clean(value, min, max, label)?;
    *total = total.saturating_add(value.chars().count());
    Ok(())
}
fn clean(value: &str, min: usize, max: usize, label: &str) -> Result<(), String> {
    let count = value.chars().count();
    if count<min || count>max || value.chars().any(|character| matches!(character, '\0'..='\u{0008}' | '\u{000B}' | '\u{000C}' | '\u{000E}'..='\u{001F}')) { return Err(format!("Artifact {label} is invalid.")); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn factual_blocks_without_evidence_fail_closed() {
        let document = ArtifactDocument {
            schema_version: 1,
            metadata: ArtifactMetadata {
                title: "Report".into(),
                subtitle: "".into(),
                author: "".into(),
                subject: "".into(),
                keywords: vec![],
                language: "en".into(),
            },
            theme: ThemeTokens::default(),
            page: PageControls::default(),
            header: None,
            footer: None,
            sections: vec![ArtifactSection {
                heading: "Findings".into(),
                page_break_before: false,
                blocks: vec![ArtifactBlock::Paragraph {
                    text: "Claim".into(),
                    style: ParagraphStyle::Body,
                    factual: true,
                    sources: vec![],
                }],
            }],
        };
        assert!(validate(&document)
            .unwrap_err()
            .contains("source and evidence"));
    }
}
