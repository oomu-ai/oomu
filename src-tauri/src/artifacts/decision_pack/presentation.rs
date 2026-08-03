use super::{
    evidence::{
        exception_evidence, margin_evidence, rate_evidence, web_evidence, CanonicalEvidence,
    },
    validate_decision_pack_analysis, DecisionPackAnalysis, MarginAssessment, RateReconciliation,
};
use crate::artifacts::presentations::*;

mod layout;
use layout::*;

const MARGIN_ROWS_PER_SLIDE: usize = 3;
const MAX_EXCEPTIONS_PER_SLIDE: usize = 4;
const MAX_EXCEPTION_CHARS_PER_SLIDE: usize = 1_100;

pub(crate) fn build_decision_presentation(
    analysis: &DecisionPackAnalysis,
) -> Result<PresentationIr, String> {
    validate_decision_pack_analysis(analysis)?;
    let mut slides = vec![title_slide(analysis), executive_slide(analysis)];
    slides.extend(rate_slides(analysis));
    slides.extend(margin_slides(analysis));
    slides.extend(exception_slides(analysis));
    slides.extend(web_slides(analysis));
    slides.push(email_slide(analysis));
    let presentation = PresentationIr {
        schema_version: PRESENTATION_IR_VERSION,
        title: analysis.title.clone(),
        locale: "en-US".to_string(),
        revision: 1,
        aspect_ratio: PresentationAspectRatio::Widescreen,
        theme: theme(),
        masters: vec![SlideMaster {
            master_id: "decision-pack-master".to_string(),
            name: "Decision pack master".to_string(),
            theme_id: "decision-pack-theme".to_string(),
            layout_ids: vec!["decision-pack-layout".to_string()],
        }],
        layouts: vec![SlideLayout {
            layout_id: "decision-pack-layout".to_string(),
            master_id: "decision-pack-master".to_string(),
            name: "Decision pack editable layout".to_string(),
            kind: SlideLayoutKind::Custom,
            placeholders: Vec::new(),
        }],
        slides,
        citations: presentation_citations(analysis),
        policy: PresentationPolicy {
            overflow: OverflowPolicy::ShrinkToFit,
            minimum_font_size_pt: 16.0,
            ..PresentationPolicy::default()
        },
        template: PresentationTemplateIdentity::default(),
    };
    validate_presentation(&presentation)?;
    Ok(presentation)
}

fn presentation_citations(analysis: &DecisionPackAnalysis) -> Vec<PresentationCitation> {
    let mut citations = Vec::new();
    let exception_groups = exception_groups(&analysis.exceptions);
    for (index, _) in analysis.rate_reconciliations.iter().enumerate() {
        citations.push(slide_citation(
            &format!("rate-citation-{}", index + 1),
            &format!("slide-rates-{}", index / 4 + 1),
            &format!("Approved rate evidence {}", index + 1),
            rate_evidence(index),
        ));
    }
    for (index, _) in analysis.margin_assessments.iter().enumerate() {
        citations.push(slide_citation(
            &format!("margin-citation-{}", index + 1),
            &format!("slide-margins-{}", index / MARGIN_ROWS_PER_SLIDE + 1),
            &format!("Approved margin evidence {}", index + 1),
            margin_evidence(index),
        ));
    }
    for (index, _) in analysis.exceptions.iter().enumerate() {
        let page = exception_groups
            .iter()
            .position(|group| group.contains(&index))
            .expect("every validated exception belongs to one presentation page");
        citations.push(slide_citation(
            &format!("exception-citation-{}", index + 1),
            &format!("slide-exceptions-{}", page + 1),
            &format!("Approved exception evidence {}", index + 1),
            exception_evidence(index),
        ));
    }
    for (index, claim) in analysis.web_claims.iter().enumerate() {
        citations.push(slide_citation(
            &format!("web-citation-{}", index + 1),
            &format!("slide-web-{}", index + 1),
            &format!("Official web source {}", index + 1),
            web_evidence(index, claim),
        ));
    }
    citations
}

fn slide_citation(
    citation_id: &str,
    slide_id: &str,
    label: &str,
    evidence: CanonicalEvidence,
) -> PresentationCitation {
    PresentationCitation {
        citation_id: citation_id.to_string(),
        slide_id: slide_id.to_string(),
        object_id: None,
        source_ref: evidence.source_ref,
        evidence_ref: evidence.evidence_ref,
        label: label.to_string(),
        locator: evidence.url,
    }
}

fn title_slide(analysis: &DecisionPackAnalysis) -> PresentationSlide {
    PresentationSlide {
        slide_id: "slide-title".to_string(),
        layout_id: "decision-pack-layout".to_string(),
        title: Some(analysis.title.clone()),
        elements: vec![
            shape(
                "title-accent",
                Frame {
                    x: 0,
                    y: 0,
                    width: 260_000,
                    height: 6_858_000,
                },
                "0B57D0",
                None,
            ),
            text_element(
                "title",
                Frame {
                    x: 900_000,
                    y: 1_250_000,
                    width: 10_300_000,
                    height: 1_450_000,
                },
                "Supplier Decision",
                54.0,
                true,
                "17365D",
                Vec::new(),
            ),
            text_element(
                "decision-title",
                Frame {
                    x: 900_000,
                    y: 2_900_000,
                    width: 10_300_000,
                    height: 1_300_000,
                },
                &analysis.title,
                22.0,
                true,
                "202124",
                Vec::new(),
            ),
            text_element(
                "subtitle",
                Frame {
                    x: 900_000,
                    y: 4_500_000,
                    width: 10_300_000,
                    height: 900_000,
                },
                "Reconciled rates • Assessed margins • Explicit conditions • Cited market evidence",
                18.0,
                false,
                "5F6368",
                Vec::new(),
            ),
        ],
        notes: SlideNotes {
            speaker_notes: format!(
                "Opening slide for the board-ready supplier decision pack: {}",
                analysis.title
            ),
            source_refs: Vec::new(),
        },
        animations: Vec::new(),
    }
}

fn executive_slide(analysis: &DecisionPackAnalysis) -> PresentationSlide {
    let snapshot = text_element(
        "decision-snapshot",
        Frame {
            x: CONTENT_X,
            y: 1_350_000,
            width: CONTENT_WIDTH,
            height: 2_050_000,
        },
        &executive_snapshot(analysis),
        18.0,
        false,
        "202124",
        Vec::new(),
    );
    let recommendation = PresentationElement {
        object_id: "recommendation".to_string(),
        frame: Frame {
            x: CONTENT_X,
            y: 3_650_000,
            width: CONTENT_WIDTH,
            height: 2_600_000,
        },
        content: ElementContent::Shape {
            geometry: ShapeGeometry::RoundedRectangle,
            fill_color: "0B57D0".to_string(),
            line_color: None,
            text: Some(text_block(
                &format!("RECOMMENDATION\n\n{}", analysis.recommendation),
                18.0,
                true,
                "FFFFFF",
            )),
        },
        provenance: Vec::new(),
    };
    content_slide(
        "slide-executive",
        "Decision at a glance",
        vec![snapshot, recommendation],
        &format!(
            "Canonical executive summary: {}\nCanonical recommendation: {}",
            analysis.executive_summary, analysis.recommendation
        ),
        Vec::new(),
    )
}

fn executive_snapshot(analysis: &DecisionPackAnalysis) -> String {
    let changed_rates = analysis
        .rate_reconciliations
        .iter()
        .filter(|rate| (rate.active_quote - rate.historical_rate).abs() > 0.005)
        .count();
    let margins_meeting_threshold = analysis
        .margin_assessments
        .iter()
        .filter(|margin| margin.margin_percent >= margin.threshold_percent)
        .count();
    format!(
        "Rate position — {changed_rates} of {} active quotes differ from history.\nMargin position — {margins_meeting_threshold} of {} assessed margins meet threshold.\nDecision conditions — {} exceptions require explicit closure.\nCurrent evidence — {} official claims qualified; {} research gaps disclosed.",
        analysis.rate_reconciliations.len(),
        analysis.margin_assessments.len(),
        analysis.exceptions.len(),
        analysis.web_claims.len(),
        analysis.research_gaps.len(),
    )
}

fn rate_slides(analysis: &DecisionPackAnalysis) -> Vec<PresentationSlide> {
    analysis
        .rate_reconciliations
        .chunks(4)
        .enumerate()
        .map(|(page, chunk)| {
            let offset = page * 4;
            let evidence = (0..chunk.len())
                .map(|index| rate_evidence(offset + index))
                .collect::<Vec<_>>();
            let table = PresentationTable {
                header_row: true,
                rows: std::iter::once(rate_header())
                    .chain(chunk.iter().map(rate_row))
                    .collect(),
            };
            let chart = PresentationChart {
                chart_type: ChartType::Column,
                title: "Historical vs active".to_string(),
                categories: chunk.iter().map(|rate| rate.name.clone()).collect(),
                series: vec![
                    ChartSeries {
                        name: "Historical".to_string(),
                        values: chunk.iter().map(|rate| rate.historical_rate).collect(),
                    },
                    ChartSeries {
                        name: "Active quote".to_string(),
                        values: chunk.iter().map(|rate| rate.active_quote).collect(),
                    },
                ],
            };
            content_slide(
                &format!("slide-rates-{}", page + 1),
                &rate_takeaway(chunk),
                vec![
                    table_element(
                        "rate-table",
                        Frame {
                            x: CONTENT_X,
                            y: 1_400_000,
                            width: CONTENT_WIDTH,
                            height: 2_550_000,
                        },
                        table,
                        &evidence,
                    ),
                    chart_element(
                        "rate-chart",
                        Frame {
                            x: CONTENT_X,
                            y: 4_100_000,
                            width: CONTENT_WIDTH,
                            height: 2_150_000,
                        },
                        chart,
                        &evidence,
                    ),
                ],
                "Every active quote is compared directly with its historical rate; variance remains formula-driven in the workbook.",
                evidence.iter().map(|item| item.source_ref.clone()).collect(),
            )
        })
        .collect()
}

fn margin_slides(analysis: &DecisionPackAnalysis) -> Vec<PresentationSlide> {
    analysis
        .margin_assessments
        .chunks(MARGIN_ROWS_PER_SLIDE)
        .enumerate()
        .map(|(page, chunk)| {
            let offset = page * MARGIN_ROWS_PER_SLIDE;
            let evidence = (0..chunk.len())
                .map(|index| margin_evidence(offset + index))
                .collect::<Vec<_>>();
            let table = PresentationTable {
                header_row: true,
                rows: std::iter::once(margin_header())
                    .chain(chunk.iter().map(margin_row))
                    .collect(),
            };
            let chart = PresentationChart {
                chart_type: ChartType::Bar,
                title: "Margin vs threshold".to_string(),
                categories: chunk.iter().map(|margin| margin.name.clone()).collect(),
                series: vec![
                    ChartSeries {
                        name: "Margin".to_string(),
                        values: chunk.iter().map(|margin| margin.margin_percent).collect(),
                    },
                    ChartSeries {
                        name: "Threshold".to_string(),
                        values: chunk.iter().map(|margin| margin.threshold_percent).collect(),
                    },
                ],
            };
            content_slide(
                &format!("slide-margins-{}", page + 1),
                &margin_takeaway(chunk),
                vec![
                    table_element(
                        "margin-table",
                        Frame {
                            x: CONTENT_X,
                            y: 1_400_000,
                            width: CONTENT_WIDTH,
                            height: 3_350_000,
                        },
                        table,
                        &evidence,
                    ),
                    chart_element(
                        "margin-chart",
                        Frame {
                            x: CONTENT_X,
                            y: 5_000_000,
                            width: CONTENT_WIDTH,
                            height: 1_300_000,
                        },
                        chart,
                        &evidence,
                    ),
                ],
                "Margins are assessed against explicit thresholds; workbook gaps are calculated, not transcribed.",
                evidence.iter().map(|item| item.source_ref.clone()).collect(),
            )
        })
        .collect()
}

fn exception_slides(analysis: &DecisionPackAnalysis) -> Vec<PresentationSlide> {
    if analysis.exceptions.is_empty() {
        return vec![content_slide(
            "slide-exceptions-clear",
            "No material conditions remain",
            vec![callout_element(
                "exceptions-clear",
                "No material exceptions were identified in the canonical analysis.",
                "E6F4EA",
                "137333",
                Vec::new(),
            )],
            "No exception records were supplied.",
            Vec::new(),
        )];
    }
    let groups = exception_groups(&analysis.exceptions);
    let page_count = groups.len();
    groups
        .into_iter()
        .enumerate()
        .map(|(page, group)| {
            let evidence = group
                .iter()
                .map(|index| exception_evidence(*index))
                .collect::<Vec<_>>();
            let exceptions = group
                .iter()
                .map(|index| format!("{}. {}", index + 1, analysis.exceptions[*index]))
                .collect::<Vec<_>>()
                .join("\n\n");
            content_slide(
                &format!("slide-exceptions-{}", page + 1),
                &format!(
                    "{} condition{} require closure",
                    analysis.exceptions.len(),
                    if analysis.exceptions.len() == 1 { "" } else { "s" }
                ),
                vec![callout_element(
                    "exceptions",
                    &exceptions,
                    "FCE8E6",
                    "A50E0E",
                    evidence.iter().map(anchor).collect(),
                )],
                &format!(
                    "Conditions page {} of {}. The decision owner should explicitly disposition each exception.",
                    page + 1,
                    page_count
                ),
                evidence.iter().map(|item| item.source_ref.clone()).collect(),
            )
        })
        .collect()
}

fn exception_groups(exceptions: &[String]) -> Vec<Vec<usize>> {
    let mut groups = Vec::new();
    let mut current = Vec::new();
    let mut current_chars = 0_usize;
    for (index, exception) in exceptions.iter().enumerate() {
        let exception_chars = exception.chars().count() + 8;
        if !current.is_empty()
            && (current.len() == MAX_EXCEPTIONS_PER_SLIDE
                || current_chars + exception_chars > MAX_EXCEPTION_CHARS_PER_SLIDE)
        {
            groups.push(std::mem::take(&mut current));
            current_chars = 0;
        }
        current.push(index);
        current_chars += exception_chars;
    }
    if !current.is_empty() {
        groups.push(current);
    }
    groups
}

fn rate_takeaway(rates: &[RateReconciliation]) -> String {
    let changed = rates
        .iter()
        .filter(|rate| (rate.active_quote - rate.historical_rate).abs() > 0.005)
        .count();
    if changed == 0 {
        "Quoted rates match history".to_string()
    } else {
        format!("{changed} of {} quoted rates changed", rates.len())
    }
}

fn margin_takeaway(margins: &[MarginAssessment]) -> String {
    let passing = margins
        .iter()
        .filter(|margin| margin.margin_percent >= margin.threshold_percent)
        .count();
    format!("{passing} of {} margins clear threshold", margins.len())
}

fn web_slides(analysis: &DecisionPackAnalysis) -> Vec<PresentationSlide> {
    if analysis.web_claims.is_empty() {
        return vec![content_slide(
            "slide-web-none",
            "No current market claim was required",
            vec![callout_element(
                "web-none",
                "No web claims were included in the canonical analysis.",
                "FEF7E0",
                "B06000",
                Vec::new(),
            )],
            "No current web evidence was supplied.",
            Vec::new(),
        )];
    }
    let mut slides = analysis
        .web_claims
        .iter()
        .enumerate()
        .map(|(index, claim)| {
            let evidence = web_evidence(index, claim);
            content_slide(
                &format!("slide-web-{}", index + 1),
                "Current evidence informs the decision",
                vec![
                    text_element(
                        "web-claim",
                        Frame {
                            x: CONTENT_X,
                            y: 1_400_000,
                            width: CONTENT_WIDTH,
                            height: 2_650_000,
                        },
                        &claim.claim,
                        20.0,
                        false,
                        "202124",
                        vec![anchor(&evidence)],
                    ),
                    text_element(
                        "web-source",
                        Frame {
                            x: CONTENT_X,
                            y: 4_250_000,
                            width: CONTENT_WIDTH,
                            height: 1_900_000,
                        },
                        &format!(
                            "{} • {}\n{} ({})\nEffective {} ({})\n{}\nAccessed {}\nEvidence {}",
                            claim.subject,
                            claim.source_title,
                            claim.authority.organization,
                            claim.authority.class.as_str(),
                            claim.effective_date,
                            claim.date_evidence_type.as_str(),
                            claim.url,
                            claim.accessed_at,
                            claim.evidence_digest,
                        ),
                        16.0,
                        false,
                        "0B57D0",
                        vec![anchor(&evidence)],
                    ),
                ],
                "Current web claim with its exact URL and access time.",
                vec![evidence.source_ref],
            )
        })
        .collect::<Vec<_>>();
    slides.extend(analysis.research_gaps.iter().enumerate().map(|(index, gap)| {
        content_slide(
            &format!("slide-research-gap-{}", index + 1),
            "An evidence gap remains explicit",
            vec![callout_element(
                "research-gap",
                &format!(
                    "{} after {} bounded attempt(s) and {} fetched page(s). No claim from this subject informed the recommendation.",
                    gap.reason.as_str(),
                    gap.attempt_count,
                    gap.page_count,
                ),
                "FEF7E0",
                "B06000",
                Vec::new(),
            )],
            "Transparent disclosure of an unresolved optional research subject.",
            Vec::new(),
        )
    }));
    slides
}

fn email_slide(analysis: &DecisionPackAnalysis) -> PresentationSlide {
    content_slide(
        "slide-email-summary",
        "Decision and next step",
        vec![callout_element(
            "email-summary",
            &analysis.email_summary,
            "E8F0FE",
            "174EA6",
            Vec::new(),
        )],
        "Editable summary content for an unsent email draft.",
        Vec::new(),
    )
}
