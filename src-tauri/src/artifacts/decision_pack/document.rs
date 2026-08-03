use super::{
    evidence::{exception_evidence, margin_evidence, rate_evidence, web_evidence},
    validate_decision_pack_analysis, DecisionPackAnalysis,
};
use crate::artifacts::{
    ArtifactBlock, ArtifactDocument, ArtifactMetadata, ArtifactSection, ArtifactSourceReference,
    PageControls, ParagraphStyle, ThemeTokens, ARTIFACT_DOCUMENT_SCHEMA_VERSION,
};

const TABLE_ROW_LIMIT: usize = 30;

pub(crate) fn build_decision_document(
    analysis: &DecisionPackAnalysis,
) -> Result<ArtifactDocument, String> {
    validate_decision_pack_analysis(analysis)?;
    let document = ArtifactDocument {
        schema_version: ARTIFACT_DOCUMENT_SCHEMA_VERSION,
        metadata: ArtifactMetadata {
            title: analysis.title.clone(),
            subtitle: "Board-ready supplier decision brief".to_string(),
            author: "OOMU".to_string(),
            subject: "Supplier recommendation, reconciliations, exceptions, and sources"
                .to_string(),
            keywords: vec![
                "supplier decision".to_string(),
                "rate reconciliation".to_string(),
                "margin assessment".to_string(),
            ],
            language: "en-US".to_string(),
        },
        theme: ThemeTokens {
            font_family: "Arial".to_string(),
            body_size_pt: 10.5,
            title_size_pt: 26.0,
            heading_color: "17365D".to_string(),
            accent_color: "0B57D0".to_string(),
            text_color: "202124".to_string(),
            background_color: "FFFFFF".to_string(),
        },
        page: PageControls {
            margin_top_in: 0.65,
            margin_right_in: 0.65,
            margin_bottom_in: 0.65,
            margin_left_in: 0.65,
            ..PageControls::default()
        },
        header: Some(analysis.title.clone()),
        footer: Some("OOMU • Supplier decision pack".to_string()),
        sections: sections(analysis),
    };
    super::super::validation::validate(&document)?;
    Ok(document)
}

fn sections(analysis: &DecisionPackAnalysis) -> Vec<ArtifactSection> {
    let mut sections = vec![ArtifactSection {
        heading: "Executive decision".to_string(),
        page_break_before: false,
        blocks: vec![
            ArtifactBlock::Paragraph {
                text: analysis.executive_summary.clone(),
                style: ParagraphStyle::Lead,
                factual: false,
                sources: Vec::new(),
            },
            ArtifactBlock::Callout {
                label: "Recommendation".to_string(),
                text: analysis.recommendation.clone(),
                factual: false,
                sources: Vec::new(),
            },
        ],
    }];
    sections.push(ArtifactSection {
        heading: "Rate reconciliation".to_string(),
        page_break_before: false,
        blocks: chunked_rate_tables(analysis),
    });
    sections.push(ArtifactSection {
        heading: "Margin assessment".to_string(),
        page_break_before: false,
        blocks: chunked_margin_tables(analysis),
    });
    sections.push(ArtifactSection {
        heading: "Exceptions".to_string(),
        page_break_before: false,
        blocks: exception_blocks(analysis),
    });
    sections.push(ArtifactSection {
        heading: "Current market evidence".to_string(),
        page_break_before: false,
        blocks: web_blocks(analysis),
    });
    sections.push(ArtifactSection {
        heading: "Unsent email summary".to_string(),
        page_break_before: false,
        blocks: vec![ArtifactBlock::Paragraph {
            text: analysis.email_summary.clone(),
            style: ParagraphStyle::Body,
            factual: false,
            sources: Vec::new(),
        }],
    });
    sections
}

fn chunked_rate_tables(analysis: &DecisionPackAnalysis) -> Vec<ArtifactBlock> {
    analysis
        .rate_reconciliations
        .chunks(TABLE_ROW_LIMIT)
        .enumerate()
        .map(|(chunk_index, chunk)| {
            let offset = chunk_index * TABLE_ROW_LIMIT;
            ArtifactBlock::Table {
                headers: vec![
                    "Supplier / item".to_string(),
                    "Historical rate".to_string(),
                    "Active quote".to_string(),
                    "Variance".to_string(),
                    "Status".to_string(),
                ],
                rows: chunk
                    .iter()
                    .map(|rate| {
                        vec![
                            rate.name.clone(),
                            format_number(rate.historical_rate),
                            format_number(rate.active_quote),
                            format_signed(rate.active_quote - rate.historical_rate),
                            display_status(&rate.status),
                        ]
                    })
                    .collect(),
                caption: "Active quotes reconciled against historical rates".to_string(),
                factual: true,
                sources: (0..chunk.len())
                    .map(|index| source_reference(rate_evidence(offset + index)))
                    .collect(),
            }
        })
        .collect()
}

fn chunked_margin_tables(analysis: &DecisionPackAnalysis) -> Vec<ArtifactBlock> {
    analysis
        .margin_assessments
        .chunks(TABLE_ROW_LIMIT)
        .enumerate()
        .flat_map(|(chunk_index, chunk)| {
            let offset = chunk_index * TABLE_ROW_LIMIT;
            let sources = (0..chunk.len())
                .map(|index| source_reference(margin_evidence(offset + index)))
                .collect::<Vec<_>>();
            [
                ArtifactBlock::Table {
                    headers: vec![
                        "Supplier".to_string(),
                        "Estimated cost".to_string(),
                        "COGS".to_string(),
                        "Margin".to_string(),
                        "Threshold".to_string(),
                        "Decision".to_string(),
                    ],
                    rows: chunk
                        .iter()
                        .map(|margin| {
                            vec![
                                margin.name.clone(),
                                format_number(margin.raw_estimated_cost),
                                format_number(margin.cogs_allocation),
                                format_percent(margin.margin_percent),
                                format_percent(margin.threshold_percent),
                                if margin.margin_percent >= margin.threshold_percent {
                                    "Meets threshold".to_string()
                                } else {
                                    "Below threshold".to_string()
                                },
                            ]
                        })
                        .collect(),
                    caption: "Reconciled margin decision".to_string(),
                    factual: true,
                    sources: sources.clone(),
                },
                ArtifactBlock::List {
                    ordered: false,
                    items: chunk
                        .iter()
                        .map(|margin| {
                            format!(
                                "{}: {} Reported and calculated margin reconcile at {}; the gap to threshold is {}.",
                                margin.name,
                                margin.notes,
                                format_percent(margin.margin_percent),
                                format_signed_percent(
                                    margin.margin_percent - margin.threshold_percent
                                ),
                            )
                        })
                        .collect(),
                    factual: true,
                    sources,
                },
            ]
        })
        .collect()
}

fn exception_blocks(analysis: &DecisionPackAnalysis) -> Vec<ArtifactBlock> {
    if analysis.exceptions.is_empty() {
        return vec![ArtifactBlock::Paragraph {
            text: "No material exceptions were identified in the canonical analysis.".to_string(),
            style: ParagraphStyle::Body,
            factual: false,
            sources: Vec::new(),
        }];
    }
    analysis
        .exceptions
        .chunks(TABLE_ROW_LIMIT)
        .enumerate()
        .map(|(chunk_index, chunk)| {
            let offset = chunk_index * TABLE_ROW_LIMIT;
            ArtifactBlock::List {
                ordered: false,
                items: chunk
                    .iter()
                    .map(|exception| format!("{exception} Status: Requires review."))
                    .collect(),
                factual: true,
                sources: (0..chunk.len())
                    .map(|index| source_reference(exception_evidence(offset + index)))
                    .collect(),
            }
        })
        .collect()
}

fn web_blocks(analysis: &DecisionPackAnalysis) -> Vec<ArtifactBlock> {
    if analysis.web_claims.is_empty() {
        return vec![ArtifactBlock::Paragraph {
            text: "No current web claims were included in the canonical analysis.".to_string(),
            style: ParagraphStyle::Body,
            factual: false,
            sources: Vec::new(),
        }];
    }
    let mut blocks = analysis
        .web_claims
        .chunks(TABLE_ROW_LIMIT)
        .enumerate()
        .map(|(chunk_index, chunk)| {
            let offset = chunk_index * TABLE_ROW_LIMIT;
            ArtifactBlock::List {
                ordered: false,
                items: chunk
                    .iter()
                    .map(|claim| {
                        format!(
                            "{}: {} Official source: {}. Authority: {} ({}). Effective: {} ({}). Accessed: {}. Evidence digest: {}.",
                            claim.subject,
                            claim.claim,
                            claim.source_title,
                            claim.authority.organization,
                            claim.authority.class.as_str(),
                            claim.effective_date,
                            claim.date_evidence_type.as_str(),
                            claim.accessed_at,
                            claim.evidence_digest,
                        )
                    })
                    .collect(),
                factual: true,
                sources: chunk
                    .iter()
                    .enumerate()
                    .map(|(index, claim)| source_reference(web_evidence(offset + index, claim)))
                    .collect(),
            }
        })
        .collect::<Vec<_>>();
    blocks.extend(
        analysis
            .web_claims
            .iter()
            .enumerate()
            .map(|(index, claim)| {
                let evidence = web_evidence(index, claim);
                ArtifactBlock::Citation {
                    label: format!("Web source {}", index + 1),
                    url: claim.url.clone(),
                    source_ref: evidence.source_ref,
                    evidence_ref: evidence.evidence_ref,
                }
            }),
    );
    if !analysis.research_gaps.is_empty() {
        blocks.push(ArtifactBlock::Callout {
            label: "Research gap".to_string(),
            text: analysis
                .research_gaps
                .iter()
                .map(|gap| {
                    format!(
                        "{}: {} after {} bounded attempt(s) and {} fetched page(s); no claim from this subject informed the recommendation.",
                        gap.subject,
                        gap.reason.as_str(),
                        gap.attempt_count,
                        gap.page_count
                    )
                })
                .collect::<Vec<_>>()
                .join(" "),
            factual: false,
            sources: Vec::new(),
        });
    }
    blocks
}

fn source_reference(evidence: super::evidence::CanonicalEvidence) -> ArtifactSourceReference {
    ArtifactSourceReference {
        source_ref: evidence.source_ref,
        evidence_ref: evidence.evidence_ref,
        url: evidence.url,
    }
}

fn format_number(value: f64) -> String {
    format!("{value:.2}")
}

fn format_signed(value: f64) -> String {
    format!("{value:+.2}")
}

fn format_percent(value: f64) -> String {
    format!("{value:.2}%")
}

fn format_signed_percent(value: f64) -> String {
    format!("{value:+.2} pp")
}

fn display_status(status: &str) -> String {
    match status.trim() {
        "PENDING_RECONCILIATION" => "Requires reconciliation".to_string(),
        "ALIGNED" => "Aligned".to_string(),
        value if value.contains('_') => value
            .split('_')
            .filter(|part| !part.is_empty())
            .map(str::to_ascii_lowercase)
            .collect::<Vec<_>>()
            .join(" "),
        value => value.to_string(),
    }
}
