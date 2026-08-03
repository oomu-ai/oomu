mod document;
mod evidence;
mod markdown;
mod model;
mod presentation;
mod validation;
mod workbook;
mod workbook_helpers;
mod workbook_source;

pub(crate) use document::build_decision_document;
pub(crate) use markdown::build_sources_markdown;
pub(crate) use model::{
    web_claim_evidence_digest, DateEvidenceType, DecisionPackAnalysis, DecisionPackArtifacts,
    MarginAssessment, RateReconciliation, ResearchGap, ResearchGapReason, SourceAuthority,
    SourceAuthorityClass, WebClaim,
};
pub(crate) use presentation::build_decision_presentation;
pub(crate) use validation::validate_decision_pack_analysis;
pub(crate) use workbook::build_decision_workbook;

pub(crate) fn build_decision_pack(
    analysis: &DecisionPackAnalysis,
) -> Result<DecisionPackArtifacts, String> {
    validate_decision_pack_analysis(analysis)?;
    Ok(DecisionPackArtifacts {
        workbook: build_decision_workbook(analysis)?,
        presentation: build_decision_presentation(analysis)?,
        document: build_decision_document(analysis)?,
        sources_markdown: build_sources_markdown(analysis)?,
    })
}

#[cfg(test)]
mod tests;
