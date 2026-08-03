use crate::artifacts::{presentations::PresentationIr, workbooks::WorkbookIr, ArtifactDocument};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DecisionPackAnalysis {
    pub(crate) title: String,
    pub(crate) executive_summary: String,
    pub(crate) recommendation: String,
    pub(crate) rate_reconciliations: Vec<RateReconciliation>,
    pub(crate) margin_assessments: Vec<MarginAssessment>,
    pub(crate) exceptions: Vec<String>,
    pub(crate) web_claims: Vec<WebClaim>,
    pub(crate) research_gaps: Vec<ResearchGap>,
    pub(crate) email_summary: String,
}

impl DecisionPackAnalysis {
    pub(crate) fn validate(&self) -> Result<(), String> {
        super::validate_decision_pack_analysis(self)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RateReconciliation {
    pub(crate) name: String,
    pub(crate) historical_rate: f64,
    pub(crate) active_quote: f64,
    pub(crate) status: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MarginAssessment {
    pub(crate) name: String,
    pub(crate) raw_estimated_cost: f64,
    pub(crate) cogs_allocation: f64,
    pub(crate) margin_percent: f64,
    pub(crate) threshold_percent: f64,
    pub(crate) notes: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WebClaim {
    pub(crate) subject: String,
    pub(crate) claim: String,
    pub(crate) source_title: String,
    pub(crate) authority: SourceAuthority,
    pub(crate) effective_date: String,
    pub(crate) date_evidence_type: DateEvidenceType,
    pub(crate) url: String,
    pub(crate) accessed_at: String,
    pub(crate) evidence_digest: String,
}

#[cfg(test)]
impl WebClaim {
    pub(crate) fn test(subject: &str, claim: &str, url: &str) -> Self {
        let source_title = "Official source".to_string();
        let authority = SourceAuthority {
            profile_id: "testGovernmentAuthority".to_string(),
            organization: "Test government authority".to_string(),
            class: SourceAuthorityClass::Government,
        };
        let effective_date = "2026-07-15".to_string();
        let date_evidence_type = DateEvidenceType::PublicationDate;
        let evidence_digest = web_claim_evidence_digest(
            subject,
            claim,
            &source_title,
            &authority,
            &effective_date,
            date_evidence_type,
            url,
        );
        Self {
            subject: subject.to_string(),
            claim: claim.to_string(),
            source_title,
            authority,
            effective_date,
            date_evidence_type,
            url: url.to_string(),
            accessed_at: "2026-07-18T12:00:00Z".to_string(),
            evidence_digest,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SourceAuthority {
    pub(crate) profile_id: String,
    pub(crate) organization: String,
    pub(crate) class: SourceAuthorityClass,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SourceAuthorityClass {
    Government,
    Intergovernmental,
    RegisteredFirstParty,
}

impl SourceAuthorityClass {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Government => "government",
            Self::Intergovernmental => "intergovernmental",
            Self::RegisteredFirstParty => "registered first-party",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DateEvidenceType {
    PublicationDate,
    ReleaseDate,
    ObservationDate,
    UpdatedDate,
}

impl DateEvidenceType {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::PublicationDate => "publication date",
            Self::ReleaseDate => "release date",
            Self::ObservationDate => "observation date",
            Self::UpdatedDate => "updated date",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ResearchGap {
    pub(crate) subject: String,
    pub(crate) reason: ResearchGapReason,
    pub(crate) attempt_count: usize,
    pub(crate) page_count: usize,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ResearchGapReason {
    EvidenceUnavailable,
    NetworkUnavailable,
}

impl ResearchGapReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::EvidenceUnavailable => "evidence unavailable",
            Self::NetworkUnavailable => "network unavailable",
        }
    }
}

pub(crate) fn web_claim_evidence_digest(
    subject: &str,
    claim: &str,
    source_title: &str,
    authority: &SourceAuthority,
    effective_date: &str,
    date_evidence_type: DateEvidenceType,
    url: &str,
) -> String {
    let fields = [
        subject,
        claim,
        source_title,
        &authority.profile_id,
        &authority.organization,
        authority.class.as_str(),
        effective_date,
        date_evidence_type.as_str(),
        url,
    ];
    let mut canonical = String::from("decision-pack-web-evidence-v1");
    for field in fields {
        canonical.push('\n');
        canonical.push_str(&field.len().to_string());
        canonical.push(':');
        canonical.push_str(field);
    }
    crate::foundation::digest::sha256_hex(canonical.as_bytes())
}

#[derive(Clone, Debug)]
pub(crate) struct DecisionPackArtifacts {
    pub(crate) workbook: WorkbookIr,
    pub(crate) presentation: PresentationIr,
    pub(crate) document: ArtifactDocument,
    pub(crate) sources_markdown: String,
}
