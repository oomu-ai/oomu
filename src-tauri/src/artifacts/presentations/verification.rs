use super::{
    exact_package_preview::render_exact_package,
    native_preview::{render_native_previews, PresentationPreviewImage},
    ooxml::{hex_digest, package_entries},
    zip::read_zip,
    ElementContent, PolicyNotice, PresentationIr, PresentationIssueSeverity,
    PresentationReviewIssue, PresentationVerificationCheck, PresentationVerificationRecord,
};
use regex::Regex;
use std::collections::{BTreeMap, HashSet};

const FORBIDDEN_PARTS: [&str; 11] = [
    "vbaproject",
    "activex",
    "embeddings/",
    "oleobject",
    "externallinks/",
    "connections.xml",
    "customui/",
    "webextensions/",
    "ctrlprops/",
    "comments/modern",
    "ink/",
];

#[derive(Clone, Debug)]
pub(crate) struct VerifiedPresentation {
    pub record: PresentationVerificationRecord,
    pub previews: Vec<PresentationPreviewImage>,
}

pub(crate) fn verify_presentation_bytes(
    bytes: &[u8],
    expected: &PresentationIr,
    policy_notices: &[PolicyNotice],
) -> Result<VerifiedPresentation, String> {
    verify_presentation(bytes, expected, policy_notices, ProjectionKind::Canonical)
}

pub(crate) fn verify_imported_presentation_bytes(
    bytes: &[u8],
    source_template: &[u8],
    expected: &PresentationIr,
    policy_notices: &[PolicyNotice],
) -> Result<VerifiedPresentation, String> {
    if !expected.template.imported {
        return Err("Imported presentation verification requires registered identity.".to_string());
    }
    verify_presentation(
        bytes,
        expected,
        policy_notices,
        ProjectionKind::Imported(source_template),
    )
}

enum ProjectionKind<'a> {
    Canonical,
    Imported(&'a [u8]),
}

fn verify_presentation(
    bytes: &[u8],
    expected: &PresentationIr,
    policy_notices: &[PolicyNotice],
    projection: ProjectionKind<'_>,
) -> Result<VerifiedPresentation, String> {
    let entries = read_safe_package(bytes)?;
    let projection_check = match projection {
        ProjectionKind::Canonical => {
            verify_projection(&entries, expected)?;
            check(
                "typed_projection_matches",
                true,
                format!(
                    "{} slides match the complete canonical typed projection.",
                    expected.slides.len()
                ),
            )
        }
        ProjectionKind::Imported(source) => {
            verify_imported_projection(&entries, source, expected)?;
            check(
                "imported_template_mapping_matches",
                true,
                "Mapped slide and notes parts match the typed projection; unrelated source masters, layouts, and metadata remain byte-identical."
                    .to_string(),
            )
        }
    };
    let editable_detail = verify_editable_objects(&entries, expected)?;
    let mut checks = structural_checks(&entries, editable_detail, projection_check);
    let structurally_verified = checks.iter().all(|check| check.passed);
    let mut issues = policy_issues(expected.revision, policy_notices);
    issues.extend(citation_issues(expected));
    issues.extend(placeholder_issues(expected));
    let semantic_check = match render_native_previews(expected) {
        Ok((semantic_previews, renderer_issues)) => {
            issues.extend(renderer_issues);
            PresentationVerificationCheck {
                code: "semantic_checks_completed".to_string(),
                passed: semantic_previews.len() == expected.slides.len(),
                detail: format!(
                    "Supplemental semantic checks covered {} of {} slides; their images are not export evidence.",
                    semantic_previews.len(),
                    expected.slides.len()
                ),
                slide_id: None,
                object_id: None,
            }
        }
        Err(error) => {
            issues.push(PresentationReviewIssue {
                issue_id: format!("semantic-{}-unavailable", expected.revision),
                revision: expected.revision,
                slide_id: None,
                code: "semantic_checks_unavailable".to_string(),
                severity: PresentationIssueSeverity::Blocker,
                message: error,
                object_id: None,
                evidence_ref: None,
            });
            PresentationVerificationCheck {
                code: "semantic_checks_completed".to_string(),
                passed: false,
                detail: "Supplemental semantic checks were unavailable; they did not authorize or replace exact-package rendering."
                    .to_string(),
                slide_id: None,
                object_id: None,
            }
        }
    };
    let semantic_verified = semantic_check.passed;
    checks.push(semantic_check);
    let slide_ids = expected
        .slides
        .iter()
        .map(|slide| slide.slide_id.clone())
        .collect::<Vec<_>>();
    let (previews, renderer, exact_check) = match render_exact_package(bytes, &slide_ids) {
        Ok(rendered) => (
            rendered.previews,
            Some(rendered.renderer_identity),
            rendered.check,
        ),
        Err(error) => {
            issues.push(PresentationReviewIssue {
                issue_id: format!("exact-package-{}-unavailable", expected.revision),
                revision: expected.revision,
                slide_id: None,
                code: "exact_package_preview_unavailable".to_string(),
                severity: PresentationIssueSeverity::Blocker,
                message: error.clone(),
                object_id: None,
                evidence_ref: None,
            });
            (
                Vec::new(),
                None,
                PresentationVerificationCheck {
                    code: "exact_package_pages_rendered".to_string(),
                    passed: false,
                    detail: error,
                    slide_id: None,
                    object_id: None,
                },
            )
        }
    };
    let visually_verified = renderer.is_some() && exact_check.passed;
    checks.push(exact_check);
    let exportable = structurally_verified
        && visually_verified
        && semantic_verified
        && !issues
            .iter()
            .any(|issue| issue.severity == PresentationIssueSeverity::Blocker);
    Ok(VerifiedPresentation {
        record: PresentationVerificationRecord {
            package_sha256: hex_digest(bytes),
            structurally_verified,
            visually_verified,
            exportable,
            checked_at_ms: crate::foundation::clock::unix_time_ms_i64(),
            renderer,
            checks,
            issues,
        },
        previews,
    })
}

pub(crate) fn read_safe_package(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let entries = read_zip(bytes)?;
    enforce_safe_package(&entries)?;
    require_parts(&entries)?;
    verify_relationship_targets(&entries)?;
    Ok(entries)
}

fn enforce_safe_package(entries: &BTreeMap<String, Vec<u8>>) -> Result<(), String> {
    for (name, bytes) in entries {
        let lowercase = name.to_ascii_lowercase();
        if FORBIDDEN_PARTS
            .iter()
            .any(|fragment| lowercase.contains(fragment))
        {
            return Err(format!(
                "Presentation package contains forbidden part {name}."
            ));
        }
        if name.ends_with(".xml") || name.ends_with(".rels") {
            let xml = std::str::from_utf8(bytes)
                .map_err(|_| format!("Presentation XML part {name} is not UTF-8."))?;
            let upper = xml.to_ascii_uppercase();
            if upper.contains("<!DOCTYPE") || upper.contains("<!ENTITY") {
                return Err(format!(
                    "Presentation XML part {name} contains entity content."
                ));
            }
            if name.ends_with(".rels") {
                let lower = xml.to_ascii_lowercase();
                if lower.contains("targetmode=\"external\"")
                    || lower.contains("targetmode='external'")
                    || lower.contains("/vbaproject")
                    || lower.contains("/activex")
                    || lower.contains("/oleobject")
                {
                    return Err(format!(
                        "Presentation relationship part {name} contains external or active content."
                    ));
                }
            }
        }
    }
    Ok(())
}

fn require_parts(entries: &BTreeMap<String, Vec<u8>>) -> Result<(), String> {
    for required in [
        "[Content_Types].xml",
        "_rels/.rels",
        "ppt/presentation.xml",
        "ppt/_rels/presentation.xml.rels",
    ] {
        if !entries.contains_key(required) {
            return Err(format!("Presentation package is missing {required}."));
        }
    }
    for prefix in [
        "ppt/slides/slide",
        "ppt/slideMasters/slideMaster",
        "ppt/slideLayouts/slideLayout",
    ] {
        if !entries.keys().any(|name| name.starts_with(prefix)) {
            return Err(format!("Presentation package has no {prefix} parts."));
        }
    }
    Ok(())
}

fn verify_projection(
    entries: &BTreeMap<String, Vec<u8>>,
    expected: &PresentationIr,
) -> Result<(), String> {
    let canonical = package_entries(expected)?;
    if entries.len() != canonical.len() {
        return Err(
            "Presentation package contains parts outside its typed projection.".to_string(),
        );
    }
    for (name, expected_bytes) in canonical {
        if entries.get(&name) != Some(&expected_bytes) {
            return Err(format!(
                "Presentation part {name} does not match its typed projection."
            ));
        }
    }
    Ok(())
}

fn verify_imported_projection(
    entries: &BTreeMap<String, Vec<u8>>,
    source_template: &[u8],
    expected: &PresentationIr,
) -> Result<(), String> {
    if hex_digest(source_template) != expected.template.fingerprint_sha256 {
        return Err("Imported source fingerprint does not match typed identity.".to_string());
    }
    let source = read_safe_package(source_template)?;
    let generated = package_entries(expected)?;
    for index in 1..=expected.slides.len() {
        for name in [
            format!("ppt/slides/slide{index}.xml"),
            format!("ppt/slides/_rels/slide{index}.xml.rels"),
            format!("ppt/notesSlides/notesSlide{index}.xml"),
        ] {
            if entries.get(&name) != generated.get(&name) {
                return Err(format!(
                    "Imported mapped part {name} does not match typed content."
                ));
            }
        }
    }
    for (name, bytes) in source {
        let replaced = (name.starts_with("ppt/slides/slide") && name.ends_with(".xml"))
            || (name.starts_with("ppt/slides/_rels/slide") && name.ends_with(".rels"))
            || (name.starts_with("ppt/notesSlides/notesSlide") && name.ends_with(".xml"));
        if !replaced && entries.get(&name) != Some(&bytes) {
            return Err(format!(
                "Imported build changed unrelated source part {name}."
            ));
        }
    }
    Ok(())
}

fn verify_relationship_targets(entries: &BTreeMap<String, Vec<u8>>) -> Result<(), String> {
    let tag = Regex::new(r"<Relationship\b[^>]*/?>").map_err(|error| error.to_string())?;
    let target =
        Regex::new(r#"\bTarget\s*=\s*["']([^"']+)["']"#).map_err(|error| error.to_string())?;
    for (name, bytes) in entries.iter().filter(|(name, _)| name.ends_with(".rels")) {
        let xml = std::str::from_utf8(bytes)
            .map_err(|_| format!("Relationship part {name} is not UTF-8."))?;
        let base = relationship_base(name)?;
        for relation in tag.find_iter(xml) {
            let captures = target
                .captures(relation.as_str())
                .ok_or_else(|| format!("Relationship in {name} has no target."))?;
            let raw = captures.get(1).unwrap().as_str();
            let resolved = normalize_target(&base, raw)?;
            if !entries.contains_key(&resolved) {
                return Err(format!(
                    "Relationship in {name} references missing part {resolved}."
                ));
            }
        }
    }
    Ok(())
}

fn relationship_base(name: &str) -> Result<String, String> {
    if name == "_rels/.rels" {
        return Ok(String::new());
    }
    let (prefix, tail) = name
        .split_once("/_rels/")
        .ok_or_else(|| format!("Relationship part path {name} is invalid."))?;
    if !tail.ends_with(".rels") {
        return Err(format!("Relationship part path {name} is invalid."));
    }
    Ok(prefix.to_string())
}

fn normalize_target(base: &str, target: &str) -> Result<String, String> {
    if target.starts_with('/') || target.contains('\\') || target.contains('#') {
        return Err("Presentation relationship target is unsafe.".to_string());
    }
    let mut components = base
        .split('/')
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    for component in target.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components
                    .pop()
                    .ok_or_else(|| "Presentation relationship escapes the package.".to_string())?;
            }
            value => components.push(value.to_string()),
        }
    }
    Ok(components.join("/"))
}

fn structural_checks(
    entries: &BTreeMap<String, Vec<u8>>,
    editable_detail: String,
    projection_check: PresentationVerificationCheck,
) -> Vec<PresentationVerificationCheck> {
    vec![
        check(
            "package_structure_valid",
            true,
            format!(
                "{} bounded package parts passed CRC and relationship checks.",
                entries.len()
            ),
        ),
        projection_check,
        check("editable_native_objects", true, editable_detail),
        check(
            "active_content_absent",
            true,
            "No active or external relationship content was found.".to_string(),
        ),
    ]
}

pub(crate) fn verify_editable_objects(
    entries: &BTreeMap<String, Vec<u8>>,
    presentation: &PresentationIr,
) -> Result<String, String> {
    let mut verified = 0_usize;
    for (slide_index, slide) in presentation.slides.iter().enumerate() {
        let path = format!("ppt/slides/slide{}.xml", slide_index + 1);
        let slide_xml = std::str::from_utf8(
            entries
                .get(&path)
                .ok_or_else(|| format!("Editable slide part {path} is missing."))?,
        )
        .map_err(|_| format!("Editable slide part {path} is not UTF-8."))?;
        for element in &slide.elements {
            let name = format!("name=\"{}\"", super::xml::attr(&element.object_id));
            let native_marker = match &element.content {
                ElementContent::TextBox { .. } | ElementContent::Shape { .. } => "<p:sp>",
                ElementContent::Image { .. } => "<p:pic>",
                ElementContent::Table { .. } => {
                    "uri=\"http://schemas.openxmlformats.org/drawingml/2006/table\""
                }
                ElementContent::Chart { .. } => {
                    "uri=\"http://schemas.openxmlformats.org/drawingml/2006/chart\""
                }
            };
            if !slide_xml.contains(&name) || !slide_xml.contains(native_marker) {
                return Err(format!(
                    "Slide {} object {} is not represented by a native editable part.",
                    slide.slide_id, element.object_id
                ));
            }
            verified += 1;
        }
        if !entries.contains_key(&format!(
            "ppt/notesSlides/notesSlide{}.xml",
            slide_index + 1
        )) {
            return Err(format!(
                "Slide {} has no editable notes part.",
                slide.slide_id
            ));
        }
    }
    Ok(format!(
        "Verified {verified} editable native objects, {} notes parts, {} layouts, and {} masters.",
        presentation.slides.len(),
        presentation.layouts.len(),
        presentation.masters.len()
    ))
}

fn policy_issues(revision: u32, notices: &[PolicyNotice]) -> Vec<PresentationReviewIssue> {
    notices
        .iter()
        .enumerate()
        .map(|(index, notice)| PresentationReviewIssue {
            issue_id: format!("policy-{revision}-{index}"),
            revision,
            slide_id: notice.slide_id.clone(),
            code: notice.code.to_string(),
            severity: match notice.code {
                "image_resolution_low" | "image_license_unknown" => {
                    PresentationIssueSeverity::Blocker
                }
                "font_substituted" | "text_shrunk_to_fit" | "animations_removed" => {
                    PresentationIssueSeverity::Warning
                }
                _ => PresentationIssueSeverity::Info,
            },
            message: notice.detail.clone(),
            object_id: notice.object_id.clone(),
            evidence_ref: None,
        })
        .collect()
}

fn citation_issues(presentation: &PresentationIr) -> Vec<PresentationReviewIssue> {
    let citations = presentation
        .citations
        .iter()
        .map(|citation| {
            (
                citation.slide_id.as_str(),
                citation.object_id.as_deref(),
                citation.source_ref.as_str(),
                citation.evidence_ref.as_str(),
            )
        })
        .collect::<HashSet<_>>();
    let mut issues = Vec::new();
    for slide in &presentation.slides {
        for element in &slide.elements {
            for anchor in &element.provenance {
                let exact = (
                    slide.slide_id.as_str(),
                    Some(element.object_id.as_str()),
                    anchor.source_ref.as_str(),
                    anchor.evidence_ref.as_str(),
                );
                let slide_level = (
                    slide.slide_id.as_str(),
                    None,
                    anchor.source_ref.as_str(),
                    anchor.evidence_ref.as_str(),
                );
                if !citations.contains(&exact) && !citations.contains(&slide_level) {
                    issues.push(PresentationReviewIssue {
                        issue_id: format!(
                            "citation-{}-{}-{}",
                            presentation.revision,
                            slide.slide_id,
                            issues.len()
                        ),
                        revision: presentation.revision,
                        slide_id: Some(slide.slide_id.clone()),
                        code: "citation_omission".to_string(),
                        severity: PresentationIssueSeverity::Blocker,
                        message: "A provenance anchor has no inspectable slide citation."
                            .to_string(),
                        object_id: Some(element.object_id.clone()),
                        evidence_ref: Some(anchor.evidence_ref.clone()),
                    });
                }
            }
        }
    }
    issues
}

fn placeholder_issues(presentation: &PresentationIr) -> Vec<PresentationReviewIssue> {
    let layouts = presentation
        .layouts
        .iter()
        .map(|layout| (layout.layout_id.as_str(), layout))
        .collect::<BTreeMap<_, _>>();
    let mut issues = Vec::new();
    for slide in &presentation.slides {
        let Some(layout) = layouts.get(slide.layout_id.as_str()) else {
            continue;
        };
        for placeholder in &layout.placeholders {
            let filled = slide.elements.iter().any(|element| {
                element.frame == placeholder.frame
                    && match (&placeholder.kind, &element.content) {
                        (
                            super::PlaceholderKind::Title
                            | super::PlaceholderKind::Subtitle
                            | super::PlaceholderKind::Body
                            | super::PlaceholderKind::Footer
                            | super::PlaceholderKind::SlideNumber,
                            ElementContent::TextBox { text },
                        ) => text
                            .paragraphs
                            .iter()
                            .flat_map(|value| &value.runs)
                            .any(|run| !run.text.trim().is_empty()),
                        (super::PlaceholderKind::Picture, ElementContent::Image { .. })
                        | (super::PlaceholderKind::Chart, ElementContent::Chart { .. })
                        | (super::PlaceholderKind::Table, ElementContent::Table { .. }) => true,
                        _ => false,
                    }
            });
            if !filled {
                issues.push(PresentationReviewIssue {
                    issue_id: format!(
                        "placeholder-{}-{}-{}",
                        presentation.revision, slide.slide_id, placeholder.placeholder_id
                    ),
                    revision: presentation.revision,
                    slide_id: Some(slide.slide_id.clone()),
                    code: "empty_placeholder".to_string(),
                    severity: PresentationIssueSeverity::Blocker,
                    message: format!(
                        "Layout placeholder {} has no matching non-empty native object.",
                        placeholder.placeholder_id
                    ),
                    object_id: Some(placeholder.placeholder_id.clone()),
                    evidence_ref: None,
                });
            }
        }
    }
    issues
}

fn check(code: &str, passed: bool, detail: String) -> PresentationVerificationCheck {
    PresentationVerificationCheck {
        code: code.to_string(),
        passed,
        detail,
        slide_id: None,
        object_id: None,
    }
}

pub fn element_is_editable(element: &ElementContent) -> bool {
    matches!(
        element,
        ElementContent::TextBox { .. }
            | ElementContent::Shape { .. }
            | ElementContent::Image { .. }
            | ElementContent::Table { .. }
            | ElementContent::Chart { .. }
    )
}
