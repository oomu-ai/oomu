use super::{
    apply_presentation_policies, hex_digest, read_safe_package, zip::write_store_zip,
    PresentationIr, PresentationRevisionScope, RevisePresentationScopeRequest,
};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationTemplateInspection {
    pub fingerprint_sha256: String,
    pub master_parts: Vec<String>,
    pub layout_parts: Vec<String>,
    pub slide_parts: Vec<String>,
    pub notes_parts: Vec<String>,
    pub metadata_parts: Vec<String>,
    pub exact_part_preservation_supported: bool,
    pub task_summary_compatible: bool,
}

pub fn revise_presentation_scope_ir(
    base: &PresentationIr,
    request: &RevisePresentationScopeRequest,
) -> Result<PresentationIr, String> {
    if request.expected_revision != base.revision {
        return Err("Presentation revision changed; reload before revising.".to_string());
    }
    let expected = base
        .revision
        .checked_add(1)
        .ok_or_else(|| "Presentation revision limit reached.".to_string())?;
    if request.presentation.revision != expected {
        return Err("Revised presentation must use the next revision number.".to_string());
    }
    if request.presentation.template != base.template {
        return Err("Scoped revision cannot replace the registered template identity.".to_string());
    }
    let targets = request
        .target_slide_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let object_targets = request
        .target_object_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    if matches!(
        request.scope,
        PresentationRevisionScope::Slide
            | PresentationRevisionScope::Element
            | PresentationRevisionScope::NarrativeSection
            | PresentationRevisionScope::Notes
            | PresentationRevisionScope::Citations
    ) && targets.is_empty()
    {
        return Err("Scoped slide revision requires at least one target slide.".to_string());
    }
    let base_ids = base
        .slides
        .iter()
        .map(|slide| slide.slide_id.as_str())
        .collect::<HashSet<_>>();
    if !targets.is_subset(&base_ids) {
        return Err("Scoped revision references an unknown slide.".to_string());
    }
    if request.scope == PresentationRevisionScope::Element {
        if object_targets.is_empty() || object_targets.len() > 32 {
            return Err("Element revision requires one to thirty-two target elements.".to_string());
        }
        let available = base
            .slides
            .iter()
            .filter(|slide| targets.contains(slide.slide_id.as_str()))
            .flat_map(|slide| {
                slide
                    .elements
                    .iter()
                    .map(|element| element.object_id.as_str())
            })
            .collect::<HashSet<_>>();
        if !object_targets.is_subset(&available) {
            return Err("Element revision references an unknown element.".to_string());
        }
    } else if !object_targets.is_empty() {
        return Err("Only an element revision may include target elements.".to_string());
    }
    enforce_scope(
        base,
        &request.presentation,
        request.scope,
        &targets,
        &object_targets,
    )?;
    Ok(apply_presentation_policies(&request.presentation)?.presentation)
}

fn enforce_scope(
    base: &PresentationIr,
    revised: &PresentationIr,
    scope: PresentationRevisionScope,
    targets: &HashSet<&str>,
    object_targets: &HashSet<&str>,
) -> Result<(), String> {
    if scope == PresentationRevisionScope::WholePresentation {
        return Ok(());
    }
    if base.title != revised.title
        || base.locale != revised.locale
        || base.aspect_ratio != revised.aspect_ratio
        || base.slides.len() != revised.slides.len()
    {
        return Err(
            "Scoped revision changed presentation-wide identity or slide order.".to_string(),
        );
    }
    match scope {
        PresentationRevisionScope::Theme => {
            if base.masters != revised.masters
                || base.layouts != revised.layouts
                || base.slides != revised.slides
                || base.citations != revised.citations
            {
                return Err("Theme revision changed content outside the theme.".to_string());
            }
        }
        PresentationRevisionScope::Slide | PresentationRevisionScope::NarrativeSection => {
            if base.theme != revised.theme
                || base.masters != revised.masters
                || base.layouts != revised.layouts
            {
                return Err("Slide revision changed theme, masters, or layouts.".to_string());
            }
            for (before, after) in base.slides.iter().zip(&revised.slides) {
                if before.slide_id != after.slide_id
                    || (!targets.contains(before.slide_id.as_str()) && before != after)
                {
                    return Err("Slide revision changed an unrelated slide.".to_string());
                }
            }
            ensure_unrelated_citations(base, revised, targets)?;
        }
        PresentationRevisionScope::Element => {
            if base.theme != revised.theme
                || base.masters != revised.masters
                || base.layouts != revised.layouts
                || base.citations != revised.citations
            {
                return Err("Element revision changed content outside its target.".to_string());
            }
            let mut changed = HashSet::new();
            for (before, after) in base.slides.iter().zip(&revised.slides) {
                if before.slide_id != after.slide_id {
                    return Err("Element revision changed slide order.".to_string());
                }
                if !targets.contains(before.slide_id.as_str()) {
                    if before != after {
                        return Err("Element revision changed an unrelated slide.".to_string());
                    }
                    continue;
                }
                if before.layout_id != after.layout_id
                    || before.notes != after.notes
                    || before.animations != after.animations
                    || before.elements.len() != after.elements.len()
                {
                    return Err("Element revision changed unrelated slide content.".to_string());
                }
                let mut slide_changed = false;
                for (before_element, after_element) in before.elements.iter().zip(&after.elements) {
                    if before_element.object_id != after_element.object_id {
                        return Err("Element revision changed element order.".to_string());
                    }
                    if before_element != after_element {
                        if !object_targets.contains(before_element.object_id.as_str()) {
                            return Err(
                                "Element revision changed an unrelated element.".to_string()
                            );
                        }
                        slide_changed = true;
                        changed.insert(before_element.object_id.as_str());
                    }
                }
                if before.title != after.title && !slide_changed {
                    return Err(
                        "Element revision changed slide metadata without its element.".to_string(),
                    );
                }
            }
            if changed.is_empty() || !changed.is_subset(object_targets) {
                return Err("Element revision did not change its selected element.".to_string());
            }
        }
        PresentationRevisionScope::Notes => {
            if base.theme != revised.theme
                || base.masters != revised.masters
                || base.layouts != revised.layouts
                || base.citations != revised.citations
            {
                return Err("Notes revision changed content outside notes.".to_string());
            }
            for (before, after) in base.slides.iter().zip(&revised.slides) {
                let mut normalized = after.clone();
                normalized.notes = before.notes.clone();
                if before.slide_id != after.slide_id
                    || before.elements != after.elements
                    || before.animations != after.animations
                    || before.layout_id != after.layout_id
                    || before.title != after.title
                    || (!targets.contains(before.slide_id.as_str()) && before.notes != after.notes)
                    || normalized != *before
                {
                    return Err("Notes revision changed unrelated slide content.".to_string());
                }
            }
        }
        PresentationRevisionScope::Citations => {
            if base.theme != revised.theme
                || base.masters != revised.masters
                || base.layouts != revised.layouts
                || base.slides != revised.slides
            {
                return Err("Citation revision changed slide content.".to_string());
            }
            ensure_unrelated_citations(base, revised, targets)?;
        }
        PresentationRevisionScope::WholePresentation => {}
    }
    Ok(())
}

fn ensure_unrelated_citations(
    base: &PresentationIr,
    revised: &PresentationIr,
    targets: &HashSet<&str>,
) -> Result<(), String> {
    let unrelated = |presentation: &PresentationIr| {
        presentation
            .citations
            .iter()
            .filter(|citation| !targets.contains(citation.slide_id.as_str()))
            .cloned()
            .collect::<Vec<_>>()
    };
    if unrelated(base) != unrelated(revised) {
        return Err("Scoped revision changed citations on an unrelated slide.".to_string());
    }
    Ok(())
}

pub fn inspect_presentation_template_bytes(
    bytes: &[u8],
) -> Result<PresentationTemplateInspection, String> {
    let entries = read_safe_package(bytes)?;
    let collect = |prefix: &str| {
        entries
            .keys()
            .filter(|name| name.starts_with(prefix) && name.ends_with(".xml"))
            .cloned()
            .collect::<Vec<_>>()
    };
    let master_parts = collect("ppt/slideMasters/slideMaster");
    let layout_parts = collect("ppt/slideLayouts/slideLayout");
    let slide_parts = collect("ppt/slides/slide");
    let notes_parts = collect("ppt/notesSlides/notesSlide");
    if master_parts.is_empty() || layout_parts.is_empty() || slide_parts.is_empty() {
        return Err(
            "Presentation template has no usable master, layout, or slide mapping.".to_string(),
        );
    }
    let task_summary_compatible = !master_parts.is_empty()
        && [
            "ppt/slideLayouts/slideLayout1.xml",
            "ppt/slideLayouts/slideLayout2.xml",
        ]
        .iter()
        .all(|name| layout_parts.iter().any(|part| part == name))
        && slide_parts == ["ppt/slides/slide1.xml", "ppt/slides/slide2.xml"]
        && notes_parts
            == [
                "ppt/notesSlides/notesSlide1.xml",
                "ppt/notesSlides/notesSlide2.xml",
            ]
        && [
            "ppt/slides/_rels/slide1.xml.rels",
            "ppt/slides/_rels/slide2.xml.rels",
        ]
        .iter()
        .all(|name| entries.contains_key(*name));
    Ok(PresentationTemplateInspection {
        fingerprint_sha256: hex_digest(bytes),
        master_parts,
        layout_parts,
        slide_parts,
        notes_parts,
        metadata_parts: entries
            .keys()
            .filter(|name| name.starts_with("docProps/") || name.starts_with("customXml/"))
            .cloned()
            .collect(),
        exact_part_preservation_supported: true,
        task_summary_compatible,
    })
}

pub fn replace_imported_slide_parts(
    source: &[u8],
    replacements: &BTreeMap<String, Vec<u8>>,
) -> Result<Vec<u8>, String> {
    if replacements.is_empty() {
        return Err("Imported presentation revision has no replacement parts.".to_string());
    }
    let mut entries = read_safe_package(source)?;
    for (name, bytes) in replacements {
        let allowed = name.starts_with("ppt/slides/slide")
            || name.starts_with("ppt/slides/_rels/slide")
            || name.starts_with("ppt/notesSlides/notesSlide")
            || name.starts_with("ppt/notesSlides/_rels/notesSlide");
        if !allowed || !name.ends_with(".xml") && !name.ends_with(".rels") {
            return Err(format!("Imported revision cannot replace part {name}."));
        }
        if !entries.contains_key(name) {
            return Err(format!("Imported source does not contain part {name}."));
        }
        std::str::from_utf8(bytes)
            .map_err(|_| format!("Replacement part {name} is not UTF-8 XML."))?;
        entries.insert(name.clone(), bytes.clone());
    }
    let output = write_store_zip(&entries)?;
    let after = read_safe_package(&output)?;
    let before = read_safe_package(source)?;
    for (name, bytes) in before {
        if !replacements.contains_key(&name) && after.get(&name) != Some(&bytes) {
            return Err(format!("Imported revision changed unrelated part {name}."));
        }
    }
    Ok(output)
}
