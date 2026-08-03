use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use std::collections::{HashMap, HashSet};

const MAX_SLIDES: usize = 1_000;
const MAX_ELEMENTS_PER_SLIDE: usize = 1_000;
const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;
const MAX_PRESENTATION_BYTES: usize = 160 * 1024 * 1024;
const MAX_ENCODED_IMAGE_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyNotice {
    pub code: &'static str,
    pub slide_id: Option<String>,
    pub object_id: Option<String>,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PolicyResult {
    pub presentation: PresentationIr,
    pub notices: Vec<PolicyNotice>,
}

pub fn apply_presentation_policies(input: &PresentationIr) -> Result<PolicyResult, String> {
    validate_header(input)?;
    let mut presentation = input.clone();
    let mut notices = Vec::new();
    validate_theme(&presentation.theme)?;
    let layout_ids = validate_masters_and_layouts(&presentation)?;
    let slide_ids = presentation
        .slides
        .iter()
        .map(|slide| slide.slide_id.clone())
        .collect::<HashSet<_>>();
    if slide_ids.len() != presentation.slides.len() {
        return Err("Presentation slide identifiers must be unique.".to_string());
    }
    let (slide_width, slide_height) = presentation.aspect_ratio.dimensions_emu();
    let allowed_fonts = presentation
        .policy
        .allowed_fonts
        .iter()
        .map(|font| font.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    let body_font = presentation.theme.fonts.body.clone();
    let policy = presentation.policy.clone();
    for slide in &mut presentation.slides {
        bounded(&slide.slide_id, 1, 256, "slide identifier")?;
        if let Some(title) = &slide.title {
            bounded(title, 1, 512, "slide title")?;
        }
        validate_notes(&slide.notes)?;
        if !layout_ids.contains(&slide.layout_id) {
            return Err(format!(
                "Slide {} references unknown layout {}.",
                slide.slide_id, slide.layout_id
            ));
        }
        if slide.elements.len() > MAX_ELEMENTS_PER_SLIDE {
            return Err(format!(
                "Slide {} contains too many elements.",
                slide.slide_id
            ));
        }
        let mut object_ids = HashSet::new();
        for element in &mut slide.elements {
            bounded(&element.object_id, 1, 256, "object identifier")?;
            if !object_ids.insert(element.object_id.clone()) {
                return Err(format!(
                    "Slide {} contains duplicate object {}.",
                    slide.slide_id, element.object_id
                ));
            }
            validate_frame(&element.frame, slide_width, slide_height)?;
            normalize_element(
                &policy,
                &allowed_fonts,
                &body_font,
                &slide.slide_id,
                element,
                &mut notices,
            )?;
            validate_provenance(&element.provenance)?;
        }
        validate_animations(slide, &object_ids)?;
        if !slide.animations.is_empty() {
            match policy.unsupported_animation {
                UnsupportedAnimationPolicy::Reject => {
                    return Err(format!(
                        "Slide {} contains unsupported animation metadata.",
                        slide.slide_id
                    ));
                }
                UnsupportedAnimationPolicy::Remove => {
                    notices.push(PolicyNotice {
                        code: "animations_removed",
                        slide_id: Some(slide.slide_id.clone()),
                        object_id: None,
                        detail: format!(
                            "Removed {} unsupported animation records.",
                            slide.animations.len()
                        ),
                    });
                    slide.animations.clear();
                }
            }
        }
    }
    validate_citations(&presentation.citations, &slide_ids, &presentation.slides)?;
    Ok(PolicyResult {
        presentation,
        notices,
    })
}

pub fn validate_presentation(input: &PresentationIr) -> Result<(), String> {
    apply_presentation_policies(input).map(|_| ())
}

fn validate_header(input: &PresentationIr) -> Result<(), String> {
    if input.schema_version != PRESENTATION_IR_VERSION {
        return Err(format!(
            "Unsupported presentation IR version {}.",
            input.schema_version
        ));
    }
    bounded(&input.title, 1, 256, "presentation title")?;
    bounded(&input.locale, 2, 35, "presentation locale")?;
    if input.revision == 0 {
        return Err("Presentation revision must be positive.".to_string());
    }
    if input.slides.is_empty() || input.slides.len() > MAX_SLIDES {
        return Err("Presentation must contain between 1 and 1,000 slides.".to_string());
    }
    if !(6.0..=24.0).contains(&input.policy.minimum_font_size_pt)
        || input.policy.minimum_image_dpi < 72
        || input.policy.minimum_image_dpi > 600
    {
        return Err("Presentation policy bounds are invalid.".to_string());
    }
    if input.policy.allowed_fonts.is_empty() {
        return Err("Presentation policy must allow at least one font.".to_string());
    }
    if input.masters.len() > 100
        || input.layouts.len() > 500
        || input.citations.len() > 50_000
        || input
            .slides
            .iter()
            .map(|slide| slide.elements.len())
            .sum::<usize>()
            > 50_000
    {
        return Err("Presentation collection budget is exceeded.".to_string());
    }
    let encoded_image_bytes = input
        .slides
        .iter()
        .flat_map(|slide| &slide.elements)
        .filter_map(|element| match &element.content {
            ElementContent::Image { image } => Some(image.bytes_base64.len()),
            _ => None,
        })
        .try_fold(0_usize, |sum, value| sum.checked_add(value))
        .ok_or_else(|| "Presentation image budget overflow.".to_string())?;
    if encoded_image_bytes > MAX_ENCODED_IMAGE_BYTES {
        return Err("Presentation encoded image budget is exceeded.".to_string());
    }
    let serialized = serde_json::to_vec(input).map_err(|error| error.to_string())?;
    if serialized.len() > MAX_PRESENTATION_BYTES {
        return Err("Presentation serialized budget is exceeded.".to_string());
    }
    let mut fonts = HashSet::new();
    if input.policy.allowed_fonts.len() > 100 {
        return Err("Presentation font policy is oversized.".to_string());
    }
    for font in &input.policy.allowed_fonts {
        bounded(font, 1, 128, "allowed font")?;
        if !fonts.insert(font.to_ascii_lowercase()) {
            return Err("Presentation allowed fonts must be unique.".to_string());
        }
    }
    validate_template_identity(&input.template)?;
    Ok(())
}

fn validate_theme(theme: &PresentationTheme) -> Result<(), String> {
    bounded(&theme.theme_id, 1, 128, "theme identifier")?;
    bounded(&theme.name, 1, 128, "theme name")?;
    bounded(&theme.fonts.heading, 1, 128, "heading font")?;
    bounded(&theme.fonts.body, 1, 128, "body font")?;
    for value in [
        &theme.colors.dark,
        &theme.colors.light,
        &theme.colors.accent_1,
        &theme.colors.accent_2,
        &theme.colors.accent_3,
        &theme.colors.accent_4,
        &theme.colors.hyperlink,
    ] {
        super::xml::color(value)?;
    }
    Ok(())
}

fn validate_masters_and_layouts(presentation: &PresentationIr) -> Result<HashSet<String>, String> {
    if presentation.masters.is_empty() || presentation.layouts.is_empty() {
        return Err("Presentation requires at least one master and layout.".to_string());
    }
    let masters = presentation
        .masters
        .iter()
        .map(|master| master.master_id.clone())
        .collect::<HashSet<_>>();
    let layouts = presentation
        .layouts
        .iter()
        .map(|layout| layout.layout_id.clone())
        .collect::<HashSet<_>>();
    if masters.len() != presentation.masters.len() || layouts.len() != presentation.layouts.len() {
        return Err("Presentation master and layout identifiers must be unique.".to_string());
    }
    let layout_map = presentation
        .layouts
        .iter()
        .map(|layout| (layout.layout_id.as_str(), layout.master_id.as_str()))
        .collect::<HashMap<_, _>>();
    for master in &presentation.masters {
        bounded(&master.master_id, 1, 128, "master identifier")?;
        bounded(&master.name, 1, 256, "master name")?;
        if master.layout_ids.len() > 100 {
            return Err(format!("Master {} has too many layouts.", master.master_id));
        }
        if master.theme_id != presentation.theme.theme_id || master.layout_ids.is_empty() {
            return Err(format!(
                "Master {} has an invalid theme or no layouts.",
                master.master_id
            ));
        }
        for layout_id in &master.layout_ids {
            if layout_map.get(layout_id.as_str()) != Some(&master.master_id.as_str()) {
                return Err(format!(
                    "Master {} references an unrelated layout {}.",
                    master.master_id, layout_id
                ));
            }
        }
    }
    for layout in &presentation.layouts {
        bounded(&layout.layout_id, 1, 128, "layout identifier")?;
        bounded(&layout.master_id, 1, 128, "layout master identifier")?;
        bounded(&layout.name, 1, 256, "layout name")?;
        if !masters.contains(&layout.master_id) {
            return Err(format!(
                "Layout {} references unknown master {}.",
                layout.layout_id, layout.master_id
            ));
        }
        let mut placeholders = HashSet::new();
        if layout.placeholders.len() > 100 {
            return Err(format!(
                "Layout {} has too many placeholders.",
                layout.layout_id
            ));
        }
        for placeholder in &layout.placeholders {
            bounded(
                &placeholder.placeholder_id,
                1,
                128,
                "placeholder identifier",
            )?;
            if !placeholders.insert(&placeholder.placeholder_id) {
                return Err(format!(
                    "Layout {} has duplicate placeholders.",
                    layout.layout_id
                ));
            }
            let (width, height) = presentation.aspect_ratio.dimensions_emu();
            validate_frame(&placeholder.frame, width, height)?;
        }
    }
    Ok(layouts)
}

fn normalize_element(
    policy: &PresentationPolicy,
    allowed_fonts: &HashSet<String>,
    body_font: &str,
    slide_id: &str,
    element: &mut PresentationElement,
    notices: &mut Vec<PolicyNotice>,
) -> Result<(), String> {
    match &mut element.content {
        ElementContent::TextBox { text } => normalize_text(
            policy,
            allowed_fonts,
            body_font,
            slide_id,
            &element.object_id,
            element.frame,
            text,
            notices,
        ),
        ElementContent::Shape {
            fill_color,
            line_color,
            text,
            ..
        } => {
            super::xml::color(fill_color)?;
            if let Some(line) = line_color {
                super::xml::color(line)?;
            }
            if let Some(text) = text {
                normalize_text(
                    policy,
                    allowed_fonts,
                    body_font,
                    slide_id,
                    &element.object_id,
                    element.frame,
                    text,
                    notices,
                )?;
            }
            Ok(())
        }
        ElementContent::Image { image } => validate_image(
            policy,
            slide_id,
            &element.object_id,
            element.frame,
            image,
            notices,
        ),
        ElementContent::Table { table } => validate_table(
            policy,
            allowed_fonts,
            body_font,
            slide_id,
            &element.object_id,
            element.frame,
            table,
            notices,
        ),
        ElementContent::Chart { chart } => validate_chart(chart),
    }
}

#[allow(clippy::too_many_arguments)]
fn normalize_text(
    policy: &PresentationPolicy,
    allowed_fonts: &HashSet<String>,
    body_font: &str,
    slide_id: &str,
    object_id: &str,
    frame: Frame,
    block: &mut TextBlock,
    notices: &mut Vec<PolicyNotice>,
) -> Result<(), String> {
    if block.paragraphs.len() > 1_000 {
        return Err(format!("Text object {object_id} has too many paragraphs."));
    }
    for run in block
        .paragraphs
        .iter_mut()
        .flat_map(|paragraph| &mut paragraph.runs)
    {
        bounded(&run.text, 0, 32_768, "text run")?;
        if !run.font_size_pt.is_finite() || !(4.0..=200.0).contains(&run.font_size_pt) {
            return Err(format!("Text object {object_id} has an invalid font size."));
        }
        super::xml::color(&run.color)?;
        if !allowed_fonts.contains(&run.font_family.to_ascii_lowercase()) {
            match policy.missing_font {
                MissingFontPolicy::Reject => {
                    return Err(format!(
                        "Text object {object_id} requests disallowed font {}.",
                        run.font_family
                    ));
                }
                MissingFontPolicy::SubstituteTheme => {
                    let original = std::mem::replace(&mut run.font_family, body_font.to_string());
                    notices.push(PolicyNotice {
                        code: "font_substituted",
                        slide_id: Some(slide_id.to_string()),
                        object_id: Some(object_id.to_string()),
                        detail: format!("Substituted {original} with {body_font}."),
                    });
                }
            }
        }
    }
    if text_overflows(block, frame) {
        match policy.overflow {
            OverflowPolicy::Reject => {
                return Err(format!(
                    "Text object {object_id} exceeds its bounded frame."
                ));
            }
            OverflowPolicy::ShrinkToFit => {
                let mut steps = 0;
                while text_overflows(block, frame) && steps < 400 {
                    let mut changed = false;
                    for run in block
                        .paragraphs
                        .iter_mut()
                        .flat_map(|paragraph| &mut paragraph.runs)
                    {
                        if run.font_size_pt > policy.minimum_font_size_pt {
                            run.font_size_pt =
                                (run.font_size_pt - 0.5).max(policy.minimum_font_size_pt);
                            changed = true;
                        }
                    }
                    if !changed {
                        break;
                    }
                    steps += 1;
                }
                if text_overflows(block, frame) {
                    return Err(format!(
                        "Text object {object_id} still overflows at the minimum font size."
                    ));
                }
                notices.push(PolicyNotice {
                    code: "text_shrunk_to_fit",
                    slide_id: Some(slide_id.to_string()),
                    object_id: Some(object_id.to_string()),
                    detail: "Reduced text size deterministically until it fit the frame."
                        .to_string(),
                });
            }
        }
    }
    Ok(())
}

fn text_overflows(block: &TextBlock, frame: Frame) -> bool {
    let paragraphs = block.paragraphs.len().max(1) as f64;
    let total_chars = block
        .paragraphs
        .iter()
        .flat_map(|paragraph| &paragraph.runs)
        .map(|run| run.text.chars().count())
        .sum::<usize>() as f64;
    let max_size = block
        .paragraphs
        .iter()
        .flat_map(|paragraph| &paragraph.runs)
        .map(|run| run.font_size_pt)
        .fold(12.0_f32, f32::max) as f64;
    let line_height = max_size * 12_700.0 * 1.25;
    let char_width = max_size * 12_700.0 * 0.55;
    let lines = (frame.height as f64 / line_height).floor().max(1.0);
    let chars_per_line = (frame.width as f64 / char_width).floor().max(1.0);
    total_chars + paragraphs * 2.0 > lines * chars_per_line
}

fn validate_image(
    policy: &PresentationPolicy,
    slide_id: &str,
    object_id: &str,
    frame: Frame,
    image: &PresentationImage,
    notices: &mut Vec<PolicyNotice>,
) -> Result<(), String> {
    bounded(&image.asset_id, 1, 256, "image asset identifier")?;
    bounded(&image.alt_text, 1, 2_000, "image alternative text")?;
    if let Some(source_url) = &image.license.source_url {
        bounded(source_url, 1, 2_048, "image source URL")?;
        if !source_url.starts_with("https://") {
            return Err(format!("Image {object_id} source URL must use HTTPS."));
        }
    }
    if let Some(attribution) = &image.license.attribution {
        bounded(attribution, 1, 2_000, "image attribution")?;
    }
    if image.width_px == 0 || image.height_px == 0 {
        return Err(format!("Image {object_id} has invalid dimensions."));
    }
    let decoded = STANDARD
        .decode(&image.bytes_base64)
        .map_err(|_| format!("Image {object_id} has invalid base64 content."))?;
    if decoded.is_empty()
        || decoded.len() > MAX_IMAGE_BYTES
        || !valid_magic(image.media_type, &decoded)
    {
        return Err(format!(
            "Image {object_id} has invalid or oversized content."
        ));
    }
    if image.license.status == ImageLicenseStatus::Unknown {
        match policy.image_license {
            ImageLicensePolicy::RequireKnown => {
                return Err(format!("Image {object_id} has unknown licensing."));
            }
            ImageLicensePolicy::AllowUnknownWithWarning => notices.push(PolicyNotice {
                code: "image_license_unknown",
                slide_id: Some(slide_id.to_string()),
                object_id: Some(object_id.to_string()),
                detail: "Image licensing requires reviewer confirmation.".to_string(),
            }),
        }
    }
    let required_width = frame.width as f64 / 914_400.0 * policy.minimum_image_dpi as f64;
    let required_height = frame.height as f64 / 914_400.0 * policy.minimum_image_dpi as f64;
    if image.width_px as f64 + 0.5 < required_width
        || image.height_px as f64 + 0.5 < required_height
    {
        notices.push(PolicyNotice {
            code: "image_resolution_low",
            slide_id: Some(slide_id.to_string()),
            object_id: Some(object_id.to_string()),
            detail: format!(
                "Image is below the configured {} DPI threshold at its placed size.",
                policy.minimum_image_dpi
            ),
        });
    }
    Ok(())
}

fn valid_magic(media_type: ImageMediaType, bytes: &[u8]) -> bool {
    match media_type {
        ImageMediaType::Png => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        ImageMediaType::Jpeg => bytes.starts_with(&[0xff, 0xd8, 0xff]),
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_table(
    policy: &PresentationPolicy,
    allowed_fonts: &HashSet<String>,
    body_font: &str,
    slide_id: &str,
    object_id: &str,
    frame: Frame,
    table: &mut PresentationTable,
    notices: &mut Vec<PolicyNotice>,
) -> Result<(), String> {
    if table.rows.is_empty() || table.rows.len() > 100 {
        return Err(format!("Table {object_id} row count is invalid."));
    }
    let columns = table.rows[0].len();
    if columns == 0 || columns > 50 || table.rows.iter().any(|row| row.len() != columns) {
        return Err(format!(
            "Table {object_id} must be rectangular and bounded."
        ));
    }
    let cell = Frame {
        x: 0,
        y: 0,
        width: frame.width / columns as i64,
        height: frame.height / table.rows.len() as i64,
    };
    for block in table.rows.iter_mut().flatten() {
        normalize_text(
            policy,
            allowed_fonts,
            body_font,
            slide_id,
            object_id,
            cell,
            block,
            notices,
        )?;
    }
    Ok(())
}

fn validate_chart(chart: &PresentationChart) -> Result<(), String> {
    bounded(&chart.title, 1, 256, "chart title")?;
    if chart.categories.is_empty()
        || chart.categories.len() > 1_000
        || chart.series.is_empty()
        || chart.series.len() > 100
    {
        return Err(format!("Chart {} has invalid dimensions.", chart.title));
    }
    for series in &chart.series {
        bounded(&series.name, 1, 256, "chart series name")?;
        if series.values.len() != chart.categories.len()
            || series.values.iter().any(|value| !value.is_finite())
        {
            return Err(format!("Chart series {} is invalid.", series.name));
        }
    }
    for category in &chart.categories {
        bounded(category, 1, 512, "chart category")?;
    }
    Ok(())
}

fn validate_provenance(anchors: &[ProvenanceAnchor]) -> Result<(), String> {
    let mut seen = HashSet::new();
    for anchor in anchors {
        bounded(&anchor.source_ref, 1, 512, "source reference")?;
        bounded(&anchor.evidence_ref, 1, 512, "evidence reference")?;
        if let Some(note) = &anchor.note {
            bounded(note, 1, 2_000, "provenance note")?;
        }
        if !seen.insert((&anchor.source_ref, &anchor.evidence_ref)) {
            return Err("Duplicate provenance anchor.".to_string());
        }
    }
    Ok(())
}

fn validate_notes(notes: &SlideNotes) -> Result<(), String> {
    bounded(&notes.speaker_notes, 0, 65_536, "speaker notes")?;
    if notes.source_refs.len() > 1_000 {
        return Err("Slide notes source reference budget is exceeded.".to_string());
    }
    let mut seen = HashSet::new();
    for source in &notes.source_refs {
        bounded(source, 1, 512, "notes source reference")?;
        if !seen.insert(source) {
            return Err("Slide notes contain duplicate source references.".to_string());
        }
    }
    Ok(())
}

fn validate_animations(
    slide: &PresentationSlide,
    object_ids: &HashSet<String>,
) -> Result<(), String> {
    if slide.animations.len() > 1_000 {
        return Err(format!("Slide {} has too many animations.", slide.slide_id));
    }
    let mut ids = HashSet::new();
    for animation in &slide.animations {
        bounded(&animation.animation_id, 1, 128, "animation identifier")?;
        bounded(&animation.object_id, 1, 256, "animation target")?;
        bounded(&animation.kind, 1, 128, "animation kind")?;
        if !ids.insert(&animation.animation_id) {
            return Err(format!(
                "Slide {} has duplicate animations.",
                slide.slide_id
            ));
        }
        if !object_ids.contains(&animation.object_id) {
            return Err(format!(
                "Animation {} references unknown object {}.",
                animation.animation_id, animation.object_id
            ));
        }
    }
    Ok(())
}

fn validate_template_identity(template: &PresentationTemplateIdentity) -> Result<(), String> {
    bounded(&template.name, 1, 256, "template name")?;
    if template.imported {
        let template_id = template.template_id.as_deref().ok_or_else(|| {
            "Imported presentation requires a registered template ID.".to_string()
        })?;
        bounded(template_id, 1, 256, "template identifier")?;
        if template.fingerprint_sha256.len() != 64
            || !template
                .fingerprint_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("Imported presentation template fingerprint is invalid.".to_string());
        }
    } else if template.template_id.is_some() || !template.fingerprint_sha256.is_empty() {
        return Err("Native presentation cannot claim an imported template identity.".to_string());
    }
    Ok(())
}

fn validate_citations(
    citations: &[PresentationCitation],
    slide_ids: &HashSet<String>,
    slides: &[PresentationSlide],
) -> Result<(), String> {
    let objects = slides
        .iter()
        .flat_map(|slide| {
            slide
                .elements
                .iter()
                .map(move |element| (slide.slide_id.as_str(), element.object_id.as_str()))
        })
        .collect::<HashSet<_>>();
    let mut ids = HashSet::new();
    for citation in citations {
        if !ids.insert(&citation.citation_id) {
            return Err("Citation identifiers must be unique.".to_string());
        }
        if !slide_ids.contains(&citation.slide_id) {
            return Err(format!(
                "Citation {} references unknown slide {}.",
                citation.citation_id, citation.slide_id
            ));
        }
        if let Some(object_id) = &citation.object_id {
            if !objects.contains(&(citation.slide_id.as_str(), object_id.as_str())) {
                return Err(format!(
                    "Citation {} references unknown object {}.",
                    citation.citation_id, object_id
                ));
            }
        }
        bounded(&citation.label, 1, 2_000, "citation label")?;
        bounded(&citation.source_ref, 1, 512, "citation source")?;
        bounded(&citation.evidence_ref, 1, 512, "citation evidence")?;
    }
    Ok(())
}

fn validate_frame(frame: &Frame, width: i64, height: i64) -> Result<(), String> {
    if frame.x < 0
        || frame.y < 0
        || frame.width <= 0
        || frame.height <= 0
        || frame.x.checked_add(frame.width).is_none()
        || frame.y.checked_add(frame.height).is_none()
        || frame.x + frame.width > width
        || frame.y + frame.height > height
    {
        return Err("Presentation object frame is outside the slide bounds.".to_string());
    }
    Ok(())
}

fn bounded(value: &str, min: usize, max: usize, label: &str) -> Result<(), String> {
    let count = value.chars().count();
    if count < min || count > max || value.chars().any(|character| character == '\0') {
        Err(format!("Invalid {label}."))
    } else {
        Ok(())
    }
}
