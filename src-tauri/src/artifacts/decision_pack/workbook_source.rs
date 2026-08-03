use super::{
    evidence::web_evidence,
    workbook::{provenance, sourced_text},
    DecisionPackAnalysis,
};
use crate::artifacts::workbooks::WorkbookCell;

pub(super) fn research_source_cells(
    analysis: &DecisionPackAnalysis,
    web_start: u32,
    gap_start: u32,
) -> Vec<WorkbookCell> {
    let mut cells = Vec::new();
    for (index, claim) in analysis.web_claims.iter().enumerate() {
        let row = web_start + index as u32;
        let evidence = web_evidence(index, claim);
        let provenance = provenance(&evidence.source_ref, &evidence.evidence_ref);
        cells.extend([
            sourced_text(row, "A", "Web claim", provenance.clone(), None),
            sourced_text(
                row,
                "B",
                &claim.source_title,
                provenance.clone(),
                Some("wrap"),
            ),
            sourced_text(row, "G", &claim.claim, provenance.clone(), Some("wrap")),
            sourced_text(row, "H", &claim.url, provenance.clone(), Some("wrap")),
            sourced_text(row, "I", &claim.accessed_at, provenance.clone(), None),
            sourced_text(row, "J", &claim.subject, provenance.clone(), Some("wrap")),
            sourced_text(
                row,
                "K",
                &claim.authority.organization,
                provenance.clone(),
                Some("wrap"),
            ),
            sourced_text(
                row,
                "L",
                claim.authority.class.as_str(),
                provenance.clone(),
                None,
            ),
            sourced_text(row, "M", &claim.effective_date, provenance.clone(), None),
            sourced_text(
                row,
                "N",
                claim.date_evidence_type.as_str(),
                provenance.clone(),
                None,
            ),
            sourced_text(row, "O", &claim.evidence_digest, provenance, Some("wrap")),
        ]);
    }
    for (index, gap) in analysis.research_gaps.iter().enumerate() {
        let row = gap_start + index as u32;
        let provenance = provenance(
            &format!("decision-pack-research-gap-{}", index + 1),
            &format!("canonical-analysis-research-gap-{}", index + 1),
        );
        cells.extend([
            sourced_text(row, "A", "Research gap", provenance.clone(), None),
            sourced_text(row, "B", &gap.subject, provenance.clone(), Some("wrap")),
            sourced_text(
                row,
                "G",
                &format!(
                    "{} after {} bounded attempt(s) and {} fetched page(s); no claim from this subject informed the recommendation.",
                    gap.reason.as_str(),
                    gap.attempt_count,
                    gap.page_count,
                ),
                provenance,
                Some("wrap"),
            ),
        ]);
    }
    cells
}
