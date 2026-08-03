use super::{
    evidence::{exception_evidence, margin_evidence, rate_evidence, web_evidence},
    validate_decision_pack_analysis, DecisionPackAnalysis,
};
use std::fmt::Write;

pub(crate) fn build_sources_markdown(analysis: &DecisionPackAnalysis) -> Result<String, String> {
    validate_decision_pack_analysis(analysis)?;
    let mut markdown = String::new();
    writeln!(markdown, "# Sources — {}\n", heading_text(&analysis.title))
        .map_err(|error| error.to_string())?;
    markdown.push_str(
        "All values below come from the validated canonical decision-pack analysis. Variances and threshold gaps are derived from those values.\n\n",
    );
    markdown.push_str("## Rate reconciliation inputs\n\n");
    markdown.push_str(
        "| Source reference | Evidence reference | Supplier / item | Historical rate | Active quote | Status |\n|---|---|---|---:|---:|---|\n",
    );
    for (index, rate) in analysis.rate_reconciliations.iter().enumerate() {
        let evidence = rate_evidence(index);
        writeln!(
            markdown,
            "| {} | {} | {} | {:.2} | {:.2} | {} |",
            evidence.source_ref,
            evidence.evidence_ref,
            table_text(&rate.name),
            rate.historical_rate,
            rate.active_quote,
            table_text(&rate.status),
        )
        .map_err(|error| error.to_string())?;
    }
    markdown.push_str("\n## Margin assessment inputs\n\n");
    markdown.push_str(
        "| Source reference | Evidence reference | Supplier | Raw estimated cost | COGS allocation | Reported margin | Calculated margin | Threshold | Notes |\n|---|---|---|---:|---:|---:|---:|---:|---|\n",
    );
    for (index, margin) in analysis.margin_assessments.iter().enumerate() {
        let evidence = margin_evidence(index);
        writeln!(
            markdown,
            "| {} | {} | {} | {:.2} | {:.2} | {:.2}% | {:.2}% | {:.2}% | {} |",
            evidence.source_ref,
            evidence.evidence_ref,
            table_text(&margin.name),
            margin.raw_estimated_cost,
            margin.cogs_allocation,
            margin.margin_percent,
            ((margin.raw_estimated_cost - margin.cogs_allocation) / margin.raw_estimated_cost)
                * 100.0,
            margin.threshold_percent,
            table_text(&margin.notes),
        )
        .map_err(|error| error.to_string())?;
    }
    markdown.push_str("\n## Exceptions\n\n");
    if analysis.exceptions.is_empty() {
        markdown.push_str("No material exceptions were included in the canonical analysis.\n");
    } else {
        for (index, exception) in analysis.exceptions.iter().enumerate() {
            let evidence = exception_evidence(index);
            writeln!(
                markdown,
                "- **{} / {}:** {}",
                evidence.source_ref,
                evidence.evidence_ref,
                line_text(exception),
            )
            .map_err(|error| error.to_string())?;
        }
    }
    markdown.push_str("\n## Current web sources\n\n");
    if analysis.web_claims.is_empty() {
        markdown.push_str("No web claims were included in the canonical analysis.\n");
    } else {
        for (index, claim) in analysis.web_claims.iter().enumerate() {
            let evidence = web_evidence(index, claim);
            writeln!(
                markdown,
                "{}. **{}** — {}  \n   Authority: {} ({})  \n   Source: [{}]({})  \n   Effective date: `{}` ({})  \n   Accessed: `{}`  \n   Evidence digest: `{}`  \n   Canonical source: `{}`  \n   Canonical evidence: `{}`",
                index + 1,
                line_text(&claim.subject),
                line_text(&claim.claim),
                line_text(&claim.authority.organization),
                claim.authority.class.as_str(),
                line_text(&claim.source_title),
                claim.url,
                inline_code(&claim.effective_date),
                claim.date_evidence_type.as_str(),
                inline_code(&claim.accessed_at),
                inline_code(&claim.evidence_digest),
                inline_code(&evidence.source_ref),
                inline_code(&evidence.evidence_ref),
            )
            .map_err(|error| error.to_string())?;
        }
    }
    markdown.push_str("\n## Research gaps\n\n");
    if analysis.research_gaps.is_empty() {
        markdown.push_str("Every required research subject was qualified.\n");
    } else {
        for gap in &analysis.research_gaps {
            writeln!(
                markdown,
                "- **{}:** {} after {} bounded attempt(s) and {} fetched page(s). No claim from this subject informed the recommendation.",
                line_text(&gap.subject),
                gap.reason.as_str(),
                gap.attempt_count,
                gap.page_count,
            )
            .map_err(|error| error.to_string())?;
        }
    }
    Ok(markdown)
}

fn heading_text(value: &str) -> String {
    value
        .replace(['\r', '\n'], " ")
        .replace('#', "\\#")
        .trim()
        .to_string()
}

fn table_text(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace(['\r', '\n'], "<br>")
}

fn line_text(value: &str) -> String {
    value
        .replace(['\r', '\n'], " ")
        .replace('\\', "\\\\")
        .replace('*', "\\*")
        .replace('_', "\\_")
}

fn inline_code(value: &str) -> String {
    value.replace('`', "'")
}
