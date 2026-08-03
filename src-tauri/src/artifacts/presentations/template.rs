use super::{
    apply_presentation_policies, inspect_presentation_template_bytes,
    ooxml::{hex_digest, package_entries, BuiltPresentation},
    verification::read_safe_package,
    zip::write_store_zip,
    ElementContent, PresentationIr,
};

pub fn build_presentation_from_registered_template(
    source: &[u8],
    input: &PresentationIr,
) -> Result<BuiltPresentation, String> {
    if !input.template.imported {
        return Err("Registered template build requires imported template identity.".to_string());
    }
    if hex_digest(source) != input.template.fingerprint_sha256 {
        return Err("Registered template fingerprint changed before build.".to_string());
    }
    if input
        .slides
        .iter()
        .flat_map(|slide| &slide.elements)
        .any(|element| {
            matches!(
                element.content,
                ElementContent::Image { .. } | ElementContent::Chart { .. }
            )
        })
    {
        return Err(
            "Imported templates currently accept native text, shapes, and tables only; asset or chart remapping is not qualified."
                .to_string(),
        );
    }
    let policy = apply_presentation_policies(input)?;
    let inspection = inspect_presentation_template_bytes(source)?;
    if inspection.slide_parts.len() != policy.presentation.slides.len()
        || inspection.notes_parts.len() != policy.presentation.slides.len()
        || inspection.master_parts.is_empty()
        || inspection.layout_parts.len() < policy.presentation.layouts.len()
    {
        return Err(
            "Imported template does not contain the required slide, notes, master, and layout mapping."
                .to_string(),
        );
    }
    let generated = package_entries(&policy.presentation)?;
    let mut entries = read_safe_package(source)?;
    for index in 1..=policy.presentation.slides.len() {
        for name in [
            format!("ppt/slides/slide{index}.xml"),
            format!("ppt/slides/_rels/slide{index}.xml.rels"),
            format!("ppt/notesSlides/notesSlide{index}.xml"),
        ] {
            if !entries.contains_key(&name) {
                return Err(format!(
                    "Imported template is missing canonical mapped part {name}."
                ));
            }
            let replacement = generated
                .get(&name)
                .ok_or_else(|| format!("Generated presentation is missing mapped part {name}."))?;
            entries.insert(name, replacement.clone());
        }
    }
    for index in 1..=policy.presentation.layouts.len() {
        if !entries.contains_key(&format!("ppt/slideLayouts/slideLayout{index}.xml")) {
            return Err("Imported template layouts are not canonically mapped.".to_string());
        }
    }
    let bytes = write_store_zip(&entries)?;
    let after = read_safe_package(&bytes)?;
    let before = read_safe_package(source)?;
    for (name, value) in before {
        let replaced = name.starts_with("ppt/slides/slide")
            || name.starts_with("ppt/slides/_rels/slide")
            || name.starts_with("ppt/notesSlides/notesSlide");
        if !replaced && after.get(&name) != Some(&value) {
            return Err(format!(
                "Imported build changed unrelated template part {name}."
            ));
        }
    }
    Ok(BuiltPresentation {
        package_sha256: hex_digest(&bytes),
        bytes,
        normalized: policy.presentation,
        policy_notices: policy.notices,
    })
}
