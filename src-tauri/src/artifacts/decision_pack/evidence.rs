use super::{DecisionPackArtifacts, WebClaim};
use crate::artifacts::{
    presentations::{validate_presentation, PresentationCitation, ProvenanceAnchor},
    workbooks::{recalculate_supported_formulas, validate_workbook, ProvenanceReference},
    ArtifactBlock,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CanonicalEvidence {
    pub(super) source_ref: String,
    pub(super) evidence_ref: String,
    pub(super) url: Option<String>,
}

pub(super) fn rate_evidence(index: usize) -> CanonicalEvidence {
    canonical("rate-reconciliation", index, None)
}

pub(super) fn margin_evidence(index: usize) -> CanonicalEvidence {
    canonical("margin-assessment", index, None)
}

pub(super) fn exception_evidence(index: usize) -> CanonicalEvidence {
    canonical("decision-exception", index, None)
}

pub(super) fn web_evidence(index: usize, claim: &WebClaim) -> CanonicalEvidence {
    canonical("web-claim", index, Some(claim.url.clone()))
}

fn canonical(kind: &str, index: usize, url: Option<String>) -> CanonicalEvidence {
    CanonicalEvidence {
        source_ref: format!("decision-pack-{kind}-{}", index + 1),
        evidence_ref: format!("canonical-analysis-{kind}-{}", index + 1),
        url,
    }
}

impl DecisionPackArtifacts {
    pub(crate) fn bind_verified_task_evidence(
        &mut self,
        source_ref: &str,
        evidence_ref: &str,
        note: &str,
    ) -> Result<(), String> {
        bounded_binding(source_ref, 512, "Decision-pack source reference")?;
        bounded_binding(evidence_ref, 512, "Decision-pack evidence reference")?;
        bounded_binding(note, 1_000, "Decision-pack evidence note")?;

        let workbook_reference = ProvenanceReference {
            source_ref: source_ref.to_string(),
            evidence_ref: evidence_ref.to_string(),
            note: Some(note.to_string()),
        };
        for cell in self
            .workbook
            .worksheets
            .iter_mut()
            .flat_map(|sheet| &mut sheet.cells)
        {
            if !cell.provenance.is_empty() {
                cell.provenance = vec![workbook_reference.clone()];
            }
        }
        recalculate_supported_formulas(&mut self.workbook)?;
        validate_workbook(&self.workbook)?;

        let presentation_reference = ProvenanceAnchor {
            source_ref: source_ref.to_string(),
            evidence_ref: evidence_ref.to_string(),
            note: Some(note.to_string()),
        };
        for slide in &mut self.presentation.slides {
            let mut has_evidence = false;
            for element in &mut slide.elements {
                if !element.provenance.is_empty() {
                    element.provenance = vec![presentation_reference.clone()];
                    has_evidence = true;
                }
            }
            if has_evidence || !slide.notes.source_refs.is_empty() {
                slide.notes.source_refs = vec![source_ref.to_string()];
            }
        }
        let web_locators = self
            .presentation
            .citations
            .iter()
            .filter_map(|citation| {
                citation
                    .locator
                    .as_ref()
                    .map(|locator| (citation.slide_id.clone(), locator.clone()))
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        self.presentation.citations = self
            .presentation
            .slides
            .iter()
            .filter(|slide| {
                slide
                    .elements
                    .iter()
                    .any(|element| !element.provenance.is_empty())
            })
            .enumerate()
            .map(|(index, slide)| PresentationCitation {
                citation_id: format!("verified-task-evidence-{}", index + 1),
                slide_id: slide.slide_id.clone(),
                object_id: None,
                source_ref: source_ref.to_string(),
                evidence_ref: evidence_ref.to_string(),
                label: "Verified decision-pack analysis".to_string(),
                locator: web_locators.get(&slide.slide_id).cloned(),
            })
            .collect();
        validate_presentation(&self.presentation)?;

        for block in self
            .document
            .sections
            .iter_mut()
            .flat_map(|section| &mut section.blocks)
        {
            match block {
                ArtifactBlock::Paragraph { sources, .. }
                | ArtifactBlock::List { sources, .. }
                | ArtifactBlock::Table { sources, .. }
                | ArtifactBlock::Callout { sources, .. } => {
                    for source in sources {
                        source.source_ref = source_ref.to_string();
                        source.evidence_ref = evidence_ref.to_string();
                    }
                }
                ArtifactBlock::Citation {
                    source_ref: citation_source,
                    evidence_ref: citation_evidence,
                    ..
                } => {
                    *citation_source = source_ref.to_string();
                    *citation_evidence = evidence_ref.to_string();
                }
                ArtifactBlock::PageBreak => {}
            }
        }
        super::super::validation::validate(&self.document)
    }
}

fn bounded_binding(value: &str, maximum: usize, label: &str) -> Result<(), String> {
    let length = value.chars().count();
    if value.trim().is_empty()
        || length > maximum
        || value.chars().any(|character| character == '\0')
    {
        Err(format!("{label} is invalid."))
    } else {
        Ok(())
    }
}
