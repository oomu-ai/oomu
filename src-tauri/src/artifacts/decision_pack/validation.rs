use super::model::{web_claim_evidence_digest, DecisionPackAnalysis};
use chrono::{DateTime, NaiveDate};
use std::collections::HashSet;

const MAX_ANALYSIS_JSON_BYTES: usize = 180 * 1024;

pub(crate) fn validate_decision_pack_analysis(
    analysis: &DecisionPackAnalysis,
) -> Result<(), String> {
    clean(&analysis.title, 1, 160, "Decision-pack title")?;
    clean(&analysis.executive_summary, 1, 1_000, "Executive summary")?;
    clean(
        &analysis.recommendation,
        1,
        1_000,
        "Decision-pack recommendation",
    )?;
    clean(&analysis.email_summary, 1, 1_600, "Email summary")?;
    if analysis.rate_reconciliations.is_empty() || analysis.rate_reconciliations.len() > 100 {
        return Err("Decision-pack rate reconciliations require 1 to 100 rows.".to_string());
    }
    if analysis.margin_assessments.is_empty() || analysis.margin_assessments.len() > 100 {
        return Err("Decision-pack margin assessments require 1 to 100 rows.".to_string());
    }
    if analysis.exceptions.len() > 100
        || analysis.web_claims.len() > 100
        || analysis.research_gaps.len() > 2
    {
        return Err("Decision-pack exception or web-claim count exceeds 100.".to_string());
    }
    let mut rate_names = HashSet::new();
    for rate in &analysis.rate_reconciliations {
        clean(&rate.name, 1, 80, "Rate-reconciliation name")?;
        clean(&rate.status, 1, 80, "Rate-reconciliation status")?;
        finite_nonnegative(rate.historical_rate, "Historical rate")?;
        finite_nonnegative(rate.active_quote, "Active quote")?;
        if !rate_names.insert(rate.name.trim().to_ascii_lowercase()) {
            return Err(format!(
                "Decision-pack rate reconciliation duplicates {}.",
                rate.name
            ));
        }
    }
    let mut margin_names = HashSet::new();
    for margin in &analysis.margin_assessments {
        clean(&margin.name, 1, 80, "Margin-assessment name")?;
        clean(&margin.notes, 0, 140, "Margin-assessment notes")?;
        finite_nonnegative(margin.raw_estimated_cost, "Raw estimated cost")?;
        finite_nonnegative(margin.cogs_allocation, "COGS allocation")?;
        if margin.raw_estimated_cost <= 0.0 || margin.cogs_allocation > margin.raw_estimated_cost {
            return Err(
                "Decision-pack COGS allocation must not exceed a positive raw estimated cost."
                    .to_string(),
            );
        }
        finite_percent(margin.margin_percent, "Margin percent")?;
        finite_percent(margin.threshold_percent, "Margin threshold")?;
        if !margin_names.insert(margin.name.trim().to_ascii_lowercase()) {
            return Err(format!(
                "Decision-pack margin assessment duplicates {}.",
                margin.name
            ));
        }
    }
    for exception in &analysis.exceptions {
        clean(exception, 1, 1_200, "Decision-pack exception")?;
    }
    let mut claim_subjects = HashSet::new();
    for claim in &analysis.web_claims {
        clean(&claim.subject, 1, 40, "Web-claim subject")?;
        clean(&claim.claim, 1, 1_200, "Web claim")?;
        clean(&claim.source_title, 1, 240, "Web-claim source title")?;
        clean(
            &claim.authority.profile_id,
            1,
            120,
            "Web-claim authority profile",
        )?;
        clean(
            &claim.authority.organization,
            1,
            240,
            "Web-claim authority organization",
        )?;
        NaiveDate::parse_from_str(&claim.effective_date, "%Y-%m-%d")
            .map_err(|_| "Web-claim effective date must be YYYY-MM-DD.".to_string())?;
        clean(&claim.url, 1, 512, "Web claim URL")?;
        clean(&claim.accessed_at, 1, 80, "Web-claim access time")?;
        DateTime::parse_from_rfc3339(&claim.accessed_at)
            .map_err(|_| "Web-claim access time must be RFC 3339.".to_string())?;
        let url = url::Url::parse(&claim.url)
            .map_err(|_| "Web claim URL must be a valid HTTP(S) URL.".to_string())?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(
                "Web claim URL must be a credential-free HTTPS URL with a host.".to_string(),
            );
        }
        clean(&claim.evidence_digest, 64, 64, "Web-claim evidence digest")?;
        if claim.evidence_digest
            != web_claim_evidence_digest(
                &claim.subject,
                &claim.claim,
                &claim.source_title,
                &claim.authority,
                &claim.effective_date,
                claim.date_evidence_type,
                &claim.url,
            )
        {
            return Err("Web-claim evidence digest does not match its provenance.".to_string());
        }
        if !claim_subjects.insert(claim.subject.to_ascii_lowercase()) {
            return Err("Decision-pack web claims duplicate a research subject.".to_string());
        }
    }
    let mut gap_subjects = HashSet::new();
    for gap in &analysis.research_gaps {
        clean(&gap.subject, 1, 40, "Research-gap subject")?;
        if gap.attempt_count == 0 || gap.attempt_count > 4 || gap.page_count > 20 {
            return Err(
                "Decision-pack research gap exceeds its bounded attempt budget.".to_string(),
            );
        }
        let subject = gap.subject.to_ascii_lowercase();
        if claim_subjects.contains(&subject) || !gap_subjects.insert(subject) {
            return Err(
                "Decision-pack research subjects cannot be both qualified and unresolved."
                    .to_string(),
            );
        }
    }
    let encoded = serde_json::to_vec(analysis).map_err(|error| error.to_string())?;
    if encoded.len() > MAX_ANALYSIS_JSON_BYTES {
        return Err("Decision-pack analysis exceeds the 180 KB input budget.".to_string());
    }
    Ok(())
}

fn finite_nonnegative(value: f64, label: &str) -> Result<(), String> {
    if !value.is_finite() || !(0.0..=1_000_000_000_000_000.0).contains(&value) {
        Err(format!("{label} must be a finite non-negative amount."))
    } else {
        Ok(())
    }
}

fn finite_percent(value: f64, label: &str) -> Result<(), String> {
    if !value.is_finite() || !(-100.0..=100.0).contains(&value) {
        Err(format!(
            "{label} must be a finite percentage from -100 to 100."
        ))
    } else {
        Ok(())
    }
}

fn clean(value: &str, minimum: usize, maximum: usize, label: &str) -> Result<(), String> {
    let length = value.chars().count();
    if length < minimum
        || length > maximum
        || value.chars().any(|character| {
            matches!(
                character,
                '\0'..='\u{0008}' | '\u{000B}' | '\u{000C}' | '\u{000E}'..='\u{001F}'
            )
        })
    {
        Err(format!("{label} is outside its safe text bounds."))
    } else {
        Ok(())
    }
}
