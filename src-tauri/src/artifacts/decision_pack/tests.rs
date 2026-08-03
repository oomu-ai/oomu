use super::*;
use crate::artifacts::{
    presentations::{
        build_presentation, validate_presentation, verify_presentation_bytes, ElementContent,
        PresentationElement, TextRun,
    },
    workbooks::{build_workbook, validate_workbook, CellValue, FormulaResult, RecalculationStatus},
    ArtifactBlock,
};

#[test]
fn canonical_analysis_round_trips_with_the_runtime_schema() {
    let analysis = fixture();
    let encoded = serde_json::to_value(&analysis).unwrap();
    assert_eq!(encoded["rateReconciliations"][0]["historicalRate"], 120.0);
    assert_eq!(encoded["marginAssessments"][0]["thresholdPercent"], 18.0);
    assert_eq!(encoded["marginAssessments"][0]["rawEstimatedCost"], 200.0);
    let decoded: DecisionPackAnalysis = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, analysis);

    let mut unknown = serde_json::to_value(&analysis).unwrap();
    unknown.as_object_mut().unwrap().insert(
        "unverifiedNarrative".to_string(),
        serde_json::json!("ignored"),
    );
    assert!(serde_json::from_value::<DecisionPackAnalysis>(unknown).is_err());
}

#[test]
fn all_outputs_build_through_the_production_ir_engines() {
    let mut analysis = fixture();
    analysis.margin_assessments[0].name = "MATRIX SHIPPING CORRIDORS".to_string();
    let artifacts = build_decision_pack(&analysis).unwrap();

    validate_workbook(&artifacts.workbook).unwrap();
    let workbook_package = build_workbook(&artifacts.workbook).unwrap();
    assert!(workbook_package.bytes.starts_with(b"PK"));
    assert!(
        workbook_package.verification.exportable,
        "the production decision-pack workbook must pass its own export gate: {:#?}",
        workbook_package.verification
    );
    assert_eq!(
        workbook_package.verification.exact_package_page_count, 5,
        "the five decision-pack sheets must render without horizontal page fragmentation"
    );
    assert_eq!(
        artifacts.workbook.recalculation.status,
        RecalculationStatus::Recalculated
    );
    assert_eq!(
        artifacts
            .workbook
            .worksheets
            .iter()
            .map(|sheet| sheet.name.as_str())
            .collect::<Vec<_>>(),
        vec![
            "Source Data",
            "Rate Reconciliation",
            "Margin Assessment",
            "Exceptions",
            "Recommendation",
        ]
    );

    validate_presentation(&artifacts.presentation).unwrap();
    let presentation_package = build_presentation(&artifacts.presentation).unwrap();
    assert!(presentation_package.bytes.starts_with(b"PK"));
    assert!(artifacts.presentation.slides.len() >= 7);
    let presentation_verification = verify_presentation_bytes(
        &presentation_package.bytes,
        &presentation_package.normalized,
        &presentation_package.policy_notices,
    )
    .unwrap();
    assert!(
        presentation_verification.record.exportable,
        "the production decision-pack presentation must pass its own export gate: {:#?}",
        presentation_verification.record
    );

    super::super::validation::validate(&artifacts.document).unwrap();
    assert!(artifacts.document.sections.iter().any(|section| {
        section.blocks.iter().any(|block| {
            matches!(
                block,
                ArtifactBlock::Citation { url, .. }
                    if url == "https://example.com/freight-index"
            )
        })
    }));
    assert!(artifacts
        .sources_markdown
        .contains("https://example.com/freight-index"));
    assert!(artifacts.sources_markdown.contains("2026-07-18T12:00:00Z"));

    let claim = &analysis.web_claims[0];
    let serialized_surfaces = [
        serde_json::to_string(&artifacts.workbook).unwrap(),
        serde_json::to_string(&artifacts.presentation).unwrap(),
        serde_json::to_string(&artifacts.document).unwrap(),
        artifacts.sources_markdown.clone(),
    ];
    for required in [
        claim.subject.as_str(),
        claim.source_title.as_str(),
        claim.authority.organization.as_str(),
        claim.authority.class.as_str(),
        claim.effective_date.as_str(),
        claim.date_evidence_type.as_str(),
        claim.evidence_digest.as_str(),
    ] {
        assert!(serialized_surfaces
            .iter()
            .all(|surface| surface.contains(required)));
    }

    for serialized in serialized_surfaces {
        assert!(serialized.contains("decision-pack-web-claim-1"));
    }
}

#[test]
fn decision_presentation_is_board_ready_and_consolidated() {
    let artifacts = build_decision_pack(&fixture()).unwrap();
    let presentation = &artifacts.presentation;
    assert_eq!(
        presentation.slides.len(),
        7,
        "the canonical fixture should read as one concise decision narrative"
    );
    assert_eq!(
        presentation
            .slides
            .iter()
            .filter(|slide| slide.slide_id.starts_with("slide-margins-"))
            .count(),
        1
    );
    assert_eq!(
        presentation
            .slides
            .iter()
            .filter(|slide| slide.slide_id.starts_with("slide-exceptions-"))
            .count(),
        1
    );
    assert!(presentation
        .citations
        .iter()
        .filter(|citation| citation.citation_id.starts_with("exception-citation-"))
        .all(|citation| citation.slide_id == "slide-exceptions-1"));

    let cover_title = presentation.slides[0]
        .elements
        .iter()
        .find(|element| element.object_id == "title")
        .unwrap();
    assert!(text_runs(cover_title)
        .iter()
        .all(|run| run.font_size_pt >= 50.0));
    for slide in presentation.slides.iter().skip(1) {
        let title = slide
            .elements
            .iter()
            .find(|element| element.object_id == "slide-title")
            .unwrap();
        assert!(text_runs(title).iter().all(|run| run.font_size_pt >= 35.0));
    }
    for run in presentation
        .slides
        .iter()
        .flat_map(|slide| &slide.elements)
        .flat_map(text_runs)
    {
        assert!(
            run.font_size_pt >= 16.0,
            "visible deck text must remain presentation-readable: {}",
            run.text
        );
    }

    let built = build_presentation(presentation).unwrap();
    assert!(built
        .policy_notices
        .iter()
        .all(|notice| notice.code != "text_shrunk_to_fit"));
    if let Some(output) = std::env::var_os("OOMU_DECISION_PACK_PPTX_QA_OUTPUT") {
        std::fs::write(output, &built.bytes).unwrap();
    }
}

#[test]
fn executive_pdf_contract_uses_readable_tables_and_human_statuses() {
    let mut analysis = fixture();
    analysis.rate_reconciliations[0].status = "PENDING_RECONCILIATION".to_string();
    let document = build_decision_document(&analysis).unwrap();
    let serialized = serde_json::to_string(&document).unwrap();
    assert!(serialized.contains("Requires reconciliation"));
    assert!(!serialized.contains("PENDING_RECONCILIATION"));

    let margin = document
        .sections
        .iter()
        .find(|section| section.heading == "Margin assessment")
        .unwrap();
    let table = margin
        .blocks
        .iter()
        .find_map(|block| match block {
            ArtifactBlock::Table { headers, rows, .. } => Some((headers, rows)),
            _ => None,
        })
        .unwrap();
    assert_eq!(table.0.len(), 6);
    assert!(table.1.iter().all(|row| row.len() == 6));
    assert!(margin.blocks.iter().any(|block| matches!(
        block,
        ArtifactBlock::List { items, .. }
            if items.iter().all(|item| item.contains("reconcile at"))
    )));
    assert!(
        !document
            .sections
            .iter()
            .find(|section| section.heading == "Current market evidence")
            .unwrap()
            .page_break_before
    );
}

#[test]
fn workbook_deltas_are_real_qualified_formulas_with_cached_results() {
    let workbook = build_decision_workbook(&fixture()).unwrap();
    let rate_sheet = workbook
        .worksheets
        .iter()
        .find(|sheet| sheet.name == "Rate Reconciliation")
        .unwrap();
    assert_formula(rate_sheet, "B2", "'Source Data'!C2", 120.0);
    assert_formula(rate_sheet, "C2", "'Source Data'!D2", 129.5);
    assert_formula(rate_sheet, "D2", "C2-B2", 9.5);

    let margin_sheet = workbook
        .worksheets
        .iter()
        .find(|sheet| sheet.name == "Margin Assessment")
        .unwrap();
    assert_formula(margin_sheet, "B2", "'Source Data'!E4", 200.0);
    assert_formula(margin_sheet, "C2", "'Source Data'!F4", 155.0);
    assert_formula(margin_sheet, "D2", "B2-C2", 45.0);
    assert_formula(margin_sheet, "E2", "D2/B2", 0.225);
    assert_formula(margin_sheet, "F2", "E2*100", 22.5);
    assert_formula(margin_sheet, "G2", "'Source Data'!C4", 22.5);
    assert_formula(margin_sheet, "H2", "G2-F2", 0.0);
    assert_formula(margin_sheet, "I2", "'Source Data'!D4", 18.0);
}

#[test]
fn validation_rejects_unsafe_or_ambiguous_canonical_data() {
    let mut duplicate = fixture();
    duplicate.rate_reconciliations[1].name = " north freight ".to_string();
    assert!(duplicate.validate().unwrap_err().contains("duplicates"));

    let mut invalid_url = fixture();
    invalid_url.web_claims[0].url = "file:///private/report.html".to_string();
    assert!(invalid_url
        .validate()
        .unwrap_err()
        .contains("credential-free HTTPS"));

    let mut non_finite = fixture();
    non_finite.rate_reconciliations[0].active_quote = f64::NAN;
    assert!(non_finite
        .validate()
        .unwrap_err()
        .contains("finite non-negative"));

    let mut bad_timestamp = fixture();
    bad_timestamp.web_claims[0].accessed_at = "yesterday".to_string();
    assert!(bad_timestamp.validate().unwrap_err().contains("RFC 3339"));
}

#[test]
fn optional_research_gap_is_disclosed_on_every_artifact_surface() {
    let mut analysis = fixture();
    analysis.web_claims[0] = WebClaim::test(
        "fuel",
        "The official diesel update reported the current weekly price.",
        "https://www.eia.gov/petroleum/gasdiesel/",
    );
    analysis.research_gaps = vec![ResearchGap {
        subject: "freight".to_string(),
        reason: ResearchGapReason::EvidenceUnavailable,
        attempt_count: 3,
        page_count: 5,
    }];
    let artifacts = build_decision_pack(&analysis).unwrap();
    for serialized in [
        serde_json::to_string(&artifacts.workbook).unwrap(),
        serde_json::to_string(&artifacts.presentation).unwrap(),
        serde_json::to_string(&artifacts.document).unwrap(),
        artifacts.sources_markdown,
    ] {
        assert!(serialized.contains("freight"));
        assert!(serialized.contains("evidence unavailable"));
    }
}

#[test]
fn optional_exception_and_web_sections_are_explicit_without_placeholder_evidence() {
    let mut analysis = fixture();
    analysis.exceptions.clear();
    analysis.web_claims.clear();
    let artifacts = build_decision_pack(&analysis).unwrap();

    assert!(artifacts.presentation.citations.iter().all(|citation| {
        !citation.citation_id.starts_with("exception-citation-")
            && !citation.citation_id.starts_with("web-citation-")
    }));
    assert!(artifacts
        .presentation
        .slides
        .iter()
        .any(|slide| slide.slide_id == "slide-web-none"));
    assert!(artifacts
        .sources_markdown
        .contains("No web claims were included"));
    assert!(!artifacts.sources_markdown.contains("https://"));
}

#[test]
fn verified_task_evidence_rebinds_every_persisted_artifact_surface() {
    let mut artifacts = build_decision_pack(&fixture()).unwrap();
    let original_digest = artifacts.workbook.recalculation.input_digest.clone();
    let source_ref = "decision_pack.analysis_completed";
    let evidence_ref = "task-event:run-verified:42";
    artifacts
        .bind_verified_task_evidence(source_ref, evidence_ref, "canonical analysis sha256 abc123")
        .unwrap();

    let workbook_refs = artifacts
        .workbook
        .worksheets
        .iter()
        .flat_map(|sheet| &sheet.cells)
        .filter_map(|cell| cell.provenance.first())
        .collect::<Vec<_>>();
    assert!(!workbook_refs.is_empty());
    assert!(workbook_refs.iter().all(|evidence| {
        evidence.source_ref == source_ref && evidence.evidence_ref == evidence_ref
    }));
    assert_ne!(
        artifacts.workbook.recalculation.input_digest,
        original_digest
    );

    let presentation_refs = artifacts
        .presentation
        .slides
        .iter()
        .flat_map(|slide| &slide.elements)
        .filter_map(|element| element.provenance.first())
        .collect::<Vec<_>>();
    assert!(!presentation_refs.is_empty());
    assert!(presentation_refs.iter().all(|evidence| {
        evidence.source_ref == source_ref && evidence.evidence_ref == evidence_ref
    }));
    assert!(artifacts.presentation.citations.iter().all(|citation| {
        citation.source_ref == source_ref && citation.evidence_ref == evidence_ref
    }));
    for slide in &artifacts.presentation.slides {
        for element in &slide.elements {
            for anchor in &element.provenance {
                assert!(artifacts.presentation.citations.iter().any(|citation| {
                    citation.slide_id == slide.slide_id
                        && citation
                            .object_id
                            .as_deref()
                            .is_none_or(|object_id| object_id == element.object_id)
                        && citation.source_ref == anchor.source_ref
                        && citation.evidence_ref == anchor.evidence_ref
                }));
            }
        }
    }
    assert!(artifacts
        .presentation
        .slides
        .iter()
        .filter(|slide| !slide.notes.source_refs.is_empty())
        .all(|slide| {
            slide.notes.source_refs.len() == 1 && slide.notes.source_refs[0] == source_ref
        }));

    for block in artifacts
        .document
        .sections
        .iter()
        .flat_map(|section| &section.blocks)
    {
        assert!(block.sources().iter().all(|evidence| {
            evidence.source_ref == source_ref && evidence.evidence_ref == evidence_ref
        }));
        if let ArtifactBlock::Citation {
            source_ref: citation_source,
            evidence_ref: citation_evidence,
            ..
        } = block
        {
            assert_eq!(citation_source, source_ref);
            assert_eq!(citation_evidence, evidence_ref);
        }
    }
}

fn assert_formula(
    sheet: &crate::artifacts::workbooks::Worksheet,
    address: &str,
    expression: &str,
    cached: f64,
) {
    let cell = sheet
        .cells
        .iter()
        .find(|cell| cell.address == address)
        .unwrap();
    assert_eq!(
        cell.value,
        CellValue::Formula {
            expression: expression.to_string(),
            cached_value: Some(FormulaResult::Number { value: cached }),
        }
    );
}

fn text_runs(element: &PresentationElement) -> Vec<&TextRun> {
    let blocks = match &element.content {
        ElementContent::TextBox { text } => vec![text],
        ElementContent::Shape {
            text: Some(text), ..
        } => vec![text],
        ElementContent::Table { table } => table.rows.iter().flatten().collect(),
        ElementContent::Shape { text: None, .. }
        | ElementContent::Image { .. }
        | ElementContent::Chart { .. } => Vec::new(),
    };
    blocks
        .into_iter()
        .flat_map(|block| &block.paragraphs)
        .flat_map(|paragraph| &paragraph.runs)
        .collect()
}

fn fixture() -> DecisionPackAnalysis {
    DecisionPackAnalysis {
        title: "Supplier Network Decision".to_string(),
        executive_summary: "Two supplier paths were reconciled against historical rates and assessed against explicit margin thresholds.".to_string(),
        recommendation: "Proceed with the North freight option subject to resolving the insurance exception and confirming the quoted validity window.".to_string(),
        rate_reconciliations: vec![
            RateReconciliation {
                name: "North freight".to_string(),
                historical_rate: 120.0,
                active_quote: 129.5,
                status: "Review variance".to_string(),
            },
            RateReconciliation {
                name: "Coastal freight".to_string(),
                historical_rate: 138.0,
                active_quote: 138.0,
                status: "Reconciled".to_string(),
            },
        ],
        margin_assessments: vec![
            MarginAssessment {
                name: "North freight".to_string(),
                raw_estimated_cost: 200.0,
                cogs_allocation: 155.0,
                margin_percent: 22.5,
                threshold_percent: 18.0,
                notes: "Headroom remains after the active quote.".to_string(),
            },
            MarginAssessment {
                name: "Coastal freight".to_string(),
                raw_estimated_cost: 250.0,
                cogs_allocation: 210.0,
                margin_percent: 16.0,
                threshold_percent: 18.0,
                notes: "Below the decision threshold.".to_string(),
            },
        ],
        exceptions: vec![
            "Insurance coverage is not explicit in the active quote.".to_string(),
            "Quote validity expires before the proposed contract start.".to_string(),
        ],
        web_claims: vec![WebClaim::test(
            "freight",
            "The referenced freight index reported elevated regional capacity pressure.",
            "https://example.com/freight-index",
        )],
        research_gaps: Vec::new(),
        email_summary: "Recommendation: proceed with North freight after the two named conditions are closed. The workbook, presentation, PDF, and source register contain the same rates, margins, exceptions, and web evidence.".to_string(),
    }
}
