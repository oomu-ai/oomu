use super::VerifiedInput;
use crate::artifacts::decision_pack::{
    DecisionPackAnalysis, MarginAssessment, RateReconciliation, ResearchGap, WebClaim,
};
use regex::Regex;
use serde_json::Value;

#[derive(Debug, Clone, Eq, PartialEq)]
struct SourcePeriod {
    source: String,
    year: i64,
    quarter: String,
}

pub(super) fn build_analysis(
    title: &str,
    inputs: &[VerifiedInput],
    web_claims: Vec<WebClaim>,
    research_gaps: Vec<ResearchGap>,
) -> Result<DecisionPackAnalysis, String> {
    let mut rates = Vec::new();
    let mut margins = Vec::new();
    let mut compliance_exceptions = Vec::new();
    let mut source_periods = Vec::new();
    for input in inputs {
        if input.path.to_ascii_lowercase().ends_with(".json") {
            if let Some(period) = collect_json_rates(&input.path, &input.content, &mut rates)? {
                source_periods.push(period);
            }
        } else {
            if let Some(period) = text_source_period(&input.path, &input.content)? {
                source_periods.push(period);
            }
            collect_text_margins(&input.content, &mut margins, &mut compliance_exceptions)?;
        }
    }
    if rates.is_empty() {
        return Err(
            "The approved inputs did not contain reconciliable historical and active supplier rates. No files were created."
                .to_string(),
        );
    }
    if margins.is_empty() {
        return Err(
            "The approved inputs did not contain vendor margins and a margin threshold. No files were created."
                .to_string(),
        );
    }
    if web_claims.is_empty() {
        return Err(
            "Official live research did not produce a citable source. No files were created."
                .to_string(),
        );
    }

    let mut exceptions = rates
        .iter()
        .filter_map(rate_exception)
        .chain(margins.iter().filter_map(margin_exception))
        .chain(margins.iter().filter_map(margin_reconciliation_exception))
        .chain(compliance_exceptions)
        .collect::<Vec<_>>();
    if let Some(exception) = source_period_exception(&source_periods) {
        exceptions.push(exception);
    }
    exceptions.sort();
    exceptions.dedup();

    let recommended = margins
        .iter()
        .filter(|assessment| assessment.margin_percent >= assessment.threshold_percent)
        .max_by(|left, right| {
            left.margin_percent
                .partial_cmp(&right.margin_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .ok_or_else(|| {
            "No vendor clears the approved margin threshold, so a positive recommendation cannot be issued."
                .to_string()
        })?;
    let live_condition = official_evidence_condition(&web_claims);
    let research_gap_condition = research_gap_condition(&research_gaps);
    let recommendation = format!(
        "Advance {} to the next diligence stage: its {:.1}% projected margin is the strongest qualifying result against the {:.1}% threshold. {} {} The live evidence is not vendor-specific and therefore does not alter the ranking derived from the approved local evidence; it materially conditions the award. If the verified external conditions invalidate quote pricing or capacity before signature, pause the award and rerun the cost and margin formulas. Resolve every listed rate and compliance exception before award.",
        recommended.name,
        recommended.margin_percent,
        recommended.threshold_percent,
        live_condition,
        research_gap_condition,
    );
    let executive_summary = format!(
        "The review reconciled {} historical-to-active supplier rates and {} strategic-vendor margins. {} exception(s) require follow-up. The recommendation is grounded in the approved local evidence and {} current official web source(s). {}",
        rates.len(),
        margins.len(),
        exceptions.len(),
        web_claims.len(),
        research_gap_condition
    );
    let email_summary = format!(
        "The supplier decision pack recommends advancing {} to the next diligence stage. Its {:.1}% projected margin leads the qualifying proposals; {} exception(s) remain open. The cited live official research evidence makes final pricing and capacity confirmation a condition of award. {}",
        recommended.name,
        recommended.margin_percent,
        exceptions.len(),
        research_gap_condition
    );
    let analysis = DecisionPackAnalysis {
        title: title.to_string(),
        executive_summary,
        recommendation,
        rate_reconciliations: rates,
        margin_assessments: margins,
        exceptions,
        web_claims,
        research_gaps,
        email_summary,
    };
    analysis.validate()?;
    Ok(analysis)
}

fn research_gap_condition(gaps: &[ResearchGap]) -> String {
    if gaps.is_empty() {
        "Every required research subject was qualified.".to_string()
    } else {
        format!(
            "No qualifying official evidence was available for {}; that research gap is disclosed and did not supply recommendation evidence.",
            gaps.iter()
                .map(|gap| gap.subject.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

fn official_evidence_condition(claims: &[WebClaim]) -> String {
    let adverse = claims.iter().any(|claim| {
        let value = claim.claim.to_ascii_lowercase();
        [
            "increase",
            "elevated",
            "disruption",
            "shortage",
            "volatile",
            "congestion",
            "pressure",
            "delay",
        ]
        .iter()
        .any(|term| value.contains(term))
    });
    if adverse {
        format!(
            "The {} current official source(s) include potentially adverse external signals.",
            claims.len()
        )
    } else {
        format!(
            "The {} current official source(s) were incorporated as live external-condition evidence.",
            claims.len()
        )
    }
}

fn collect_json_rates(
    source: &str,
    content: &str,
    rates: &mut Vec<RateReconciliation>,
) -> Result<Option<SourcePeriod>, String> {
    let value = serde_json::from_str::<Value>(content).map_err(|_| {
        "An approved JSON input is not valid JSON. No files were created.".to_string()
    })?;
    collect_rate_arrays(&value, rates);
    let period = value.as_object().and_then(|object| {
        let year = object.get("audit_year")?.as_i64()?;
        let quarter = normalized_quarter(object.get("quarter")?.as_str()?)?;
        Some(SourcePeriod {
            source: source.to_string(),
            year,
            quarter,
        })
    });
    Ok(period)
}

fn text_source_period(source: &str, content: &str) -> Result<Option<SourcePeriod>, String> {
    let document_id =
        Regex::new(r"(?im)^\s*Document\s+ID\s*:\s*[^\r\n]*?(20\d{2})-(Q[1-4])(?:-|\s|$)")
            .map_err(|error| error.to_string())?;
    Ok(document_id.captures(content).and_then(|captures| {
        Some(SourcePeriod {
            source: source.to_string(),
            year: captures.get(1)?.as_str().parse().ok()?,
            quarter: normalized_quarter(captures.get(2)?.as_str())?,
        })
    }))
}

fn normalized_quarter(value: &str) -> Option<String> {
    let quarter = value.trim().to_ascii_uppercase();
    matches!(quarter.as_str(), "Q1" | "Q2" | "Q3" | "Q4").then_some(quarter)
}

fn source_period_exception(periods: &[SourcePeriod]) -> Option<String> {
    let first = periods.first()?;
    let conflicting = periods
        .iter()
        .skip(1)
        .find(|period| period.year != first.year || period.quarter != first.quarter)?;
    Some(format!(
        "Source-period mismatch: {} identifies {} {}, while {} identifies {} {}. Reconcile the reporting period before award.",
        first.source,
        first.quarter,
        first.year,
        conflicting.source,
        conflicting.quarter,
        conflicting.year
    ))
}

fn collect_rate_arrays(value: &Value, rates: &mut Vec<RateReconciliation>) {
    match value {
        Value::Array(values) => {
            for value in values {
                if let Some(rate) = rate_from_object(value) {
                    rates.push(rate);
                } else {
                    collect_rate_arrays(value, rates);
                }
            }
        }
        Value::Object(values) => {
            if let Some(rate) = rate_from_object(value) {
                rates.push(rate);
            } else {
                for child in values.values() {
                    collect_rate_arrays(child, rates);
                }
            }
        }
        _ => {}
    }
}

fn rate_from_object(value: &Value) -> Option<RateReconciliation> {
    let object = value.as_object()?;
    let name = object.get("name")?.as_str()?.trim();
    let historical_rate = numeric_field(
        object
            .get("historical_settled_rate")
            .or_else(|| object.get("historical_rate"))?,
    )?;
    let active_quote = numeric_field(object.get("active_quote")?)?;
    let status = object
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("RECONCILIATION_REQUIRED")
        .trim();
    (!name.is_empty()).then(|| RateReconciliation {
        name: name.to_string(),
        historical_rate,
        active_quote,
        status: status.to_string(),
    })
}

fn numeric_field(value: &Value) -> Option<f64> {
    value.as_f64().or_else(|| {
        value
            .as_str()?
            .replace(['$', ',', '%'], "")
            .trim()
            .parse::<f64>()
            .ok()
    })
}

fn collect_text_margins(
    content: &str,
    margins: &mut Vec<MarginAssessment>,
    compliance_exceptions: &mut Vec<String>,
) -> Result<(), String> {
    let threshold = capture_number(
        content,
        r"(?im)^\s*Target\s+Margin\s+Threshold\s*:\s*([0-9]+(?:\.[0-9]+)?)\s*%",
    )
    .ok_or_else(|| {
        "An approved proposal input has no target margin threshold. No files were created."
            .to_string()
    })?;
    let vendor = Regex::new(r"(?im)^\s*---\s*VENDOR\s+[^:]+:\s*([^\r\n-]+?)\s*---\s*$")
        .map_err(|error| error.to_string())?;
    let margin = Regex::new(r"(?im)^\s*Gross\s+Projected\s+Margin\s*:\s*([0-9]+(?:\.[0-9]+)?)\s*%")
        .map_err(|error| error.to_string())?;
    let raw_cost =
        Regex::new(r"(?im)^\s*Raw\s+Estimated\s+Cost\s*:\s*\$?\s*([0-9][0-9,]*(?:\.[0-9]+)?)")
            .map_err(|error| error.to_string())?;
    let cogs = Regex::new(
        r"(?im)^\s*Cost\s+of\s+Goods\s+Sold\s*\(COGS\)\s+Allocation\s*:\s*\$?\s*([0-9][0-9,]*(?:\.[0-9]+)?)",
    )
    .map_err(|error| error.to_string())?;
    let compliance = Regex::new(r"(?im)^\s*Compliance\s+Status\s*:\s*([^\r\n]+)")
        .map_err(|error| error.to_string())?;
    let headings = vendor
        .captures_iter(content)
        .filter_map(|captures| {
            Some((
                captures.get(0)?.start(),
                captures.get(0)?.end(),
                captures.get(1)?.as_str().trim().to_string(),
            ))
        })
        .collect::<Vec<_>>();
    for (index, (_, body_start, name)) in headings.iter().enumerate() {
        let body_end = headings
            .get(index + 1)
            .map(|heading| heading.0)
            .unwrap_or(content.len());
        let body = content[*body_start..body_end].trim();
        let Some(margin_percent) = margin
            .captures(body)
            .and_then(|captures| captures.get(1))
            .and_then(|value| value.as_str().parse::<f64>().ok())
        else {
            continue;
        };
        let raw_estimated_cost = capture_currency(&raw_cost, body).ok_or_else(|| {
            format!(
                "Approved proposal evidence for {name} has no raw estimated cost. No files were created."
            )
        })?;
        let cogs_allocation = capture_currency(&cogs, body).ok_or_else(|| {
            format!(
                "Approved proposal evidence for {name} has no COGS allocation. No files were created."
            )
        })?;
        let notes = compliance
            .captures(body)
            .and_then(|captures| captures.get(1))
            .map(|value| value.as_str().trim().to_string())
            .unwrap_or_else(|| "Compliance status not supplied.".to_string());
        let lower = notes.to_ascii_lowercase();
        if lower.contains("requires")
            || lower.contains("not ")
            || lower.contains("pending")
            || lower.contains("separate")
        {
            compliance_exceptions.push(format!("{name}: {notes}"));
        }
        margins.push(MarginAssessment {
            name: name.clone(),
            raw_estimated_cost,
            cogs_allocation,
            margin_percent,
            threshold_percent: threshold,
            notes,
        });
    }
    Ok(())
}

fn capture_currency(pattern: &Regex, content: &str) -> Option<f64> {
    pattern
        .captures(content)?
        .get(1)?
        .as_str()
        .replace(',', "")
        .parse::<f64>()
        .ok()
}

fn capture_number(content: &str, pattern: &str) -> Option<f64> {
    Regex::new(pattern)
        .ok()?
        .captures(content)?
        .get(1)?
        .as_str()
        .parse::<f64>()
        .ok()
}

fn rate_exception(rate: &RateReconciliation) -> Option<String> {
    let variance = rate.active_quote - rate.historical_rate;
    (variance.abs() > f64::EPSILON).then(|| {
        let direction = if variance > 0.0 { "above" } else { "below" };
        format!(
            "{}: active quote is ${:.2} {} the historical settled rate.",
            rate.name,
            variance.abs(),
            direction
        )
    })
}

fn margin_exception(margin: &MarginAssessment) -> Option<String> {
    (margin.margin_percent < margin.threshold_percent).then(|| {
        format!(
            "{}: {:.1}% projected margin is below the {:.1}% threshold.",
            margin.name, margin.margin_percent, margin.threshold_percent
        )
    })
}

fn margin_reconciliation_exception(margin: &MarginAssessment) -> Option<String> {
    let calculated =
        ((margin.raw_estimated_cost - margin.cogs_allocation) / margin.raw_estimated_cost) * 100.0;
    let variance = margin.margin_percent - calculated;
    (variance.abs() > 0.05).then(|| {
        format!(
            "{}: reported {:.1}% margin differs from the {:.1}% margin calculated from raw cost and COGS by {:+.1} percentage points.",
            margin.name, margin.margin_percent, calculated, variance
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rate_and_margin_evidence_without_model_authorship() {
        let inputs = vec![
            VerifiedInput::test("rates.json", r#"{"suppliers":[{"name":"Apex","historical_settled_rate":45000,"active_quote":46500,"status":"PENDING"}]}"#),
            VerifiedInput::test("proposals.txt", "Target Margin Threshold: 65%\n--- VENDOR A: MATRIX SHIPPING ---\nRaw Estimated Cost: $38,000.00\nCost of Goods Sold (COGS) Allocation: $11,020.00\nGross Projected Margin: 71.0%\nCompliance Status: Fully certified."),
        ];
        let analysis = build_analysis(
            "Supplier decision",
            &inputs,
            vec![WebClaim::test(
                "freight",
                "Official freight conditions were reviewed.",
                "https://www.bts.gov/example",
            )],
            Vec::new(),
        )
        .unwrap();
        assert_eq!(analysis.rate_reconciliations[0].active_quote, 46_500.0);
        assert_eq!(analysis.margin_assessments[0].margin_percent, 71.0);
        assert_eq!(analysis.margin_assessments[0].raw_estimated_cost, 38_000.0);
        assert_eq!(analysis.margin_assessments[0].cogs_allocation, 11_020.0);
        assert!(analysis.recommendation.contains("MATRIX SHIPPING"));
        assert!(analysis.exceptions[0].contains("$1500.00"));
        assert!(!analysis
            .recommendation
            .to_ascii_lowercase()
            .contains("fixture"));
        assert!(analysis
            .recommendation
            .contains("ranking derived from the approved local evidence"));
        assert!(analysis
            .email_summary
            .contains("cited live official research evidence"));
        assert!(!analysis.email_summary.contains("fuel and freight evidence"));
        assert!(!analysis.recommendation.contains("fuel or freight"));
        assert!(!analysis.recommendation.contains("fuel and freight"));
        assert!(!analysis.recommendation.contains("market conditions"));
    }

    #[test]
    fn official_condition_copy_does_not_invent_a_research_subject() {
        for claim in [
            "The reported conditions were stable.",
            "The reported conditions included elevated delays.",
        ] {
            let copy = official_evidence_condition(&[WebClaim::test(
                "freight",
                claim,
                "https://www.bts.gov/example",
            )]);
            assert!(!copy.contains("fuel"));
            assert!(!copy.contains("freight"));
            assert!(!copy.contains("market"));
            assert!(copy.contains("official source"));
        }
    }

    #[test]
    fn conflicting_source_quarters_are_preserved_as_an_explicit_exception() {
        let inputs = vec![
            VerifiedInput::test(
                "supplier_proposals.json",
                r#"{"audit_year":2026,"quarter":"Q2","suppliers":[{"name":"Apex","historical_settled_rate":45000,"active_quote":46500,"status":"PENDING"}]}"#,
            ),
            VerifiedInput::test(
                "q3_strategic_vendor_proposals.txt",
                "Document ID: RFP-2026-Q3-LOG\nTarget Margin Threshold: 65%\n--- VENDOR A: MATRIX SHIPPING ---\nRaw Estimated Cost: $38,000.00\nCost of Goods Sold (COGS) Allocation: $11,020.00\nGross Projected Margin: 71.0%\nCompliance Status: Fully certified.",
            ),
        ];
        let analysis = build_analysis(
            "Supplier decision",
            &inputs,
            vec![WebClaim::test(
                "freight",
                "Official freight conditions [source date 2026-07-15] were reviewed.",
                "https://www.bts.gov/example",
            )],
            Vec::new(),
        )
        .unwrap();
        assert!(analysis.exceptions.iter().any(|exception| {
            exception.contains("Source-period mismatch")
                && exception.contains("Q2 2026")
                && exception.contains("Q3 2026")
        }));
    }
}
