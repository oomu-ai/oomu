use crate::{
    foundation::digest::sha256_hex,
    p0_contracts::EvidenceClass,
    shield_gate::{CommandStatus, ExecuteCommandResponse},
    tools::{
        task_runtime::{record_event, require_agent_runtime_task},
        task_tool_runtime::{
            TaskToolApprovalTier, TaskToolExecutionContext, TaskToolFuture, TaskToolMetadata,
            TaskToolRegistration, TaskToolRiskTier, TaskToolValidation,
        },
    },
};
use chrono::{DateTime, NaiveDate};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use url::Url;

const OPERATION: &str = "compose_evidence_report";
const MAX_REPORT_BYTES: usize = 512 * 1024;
const MAX_OFFICIAL_RECEIPTS: usize = 16;
const MAX_SUPPLIERS: usize = 10_000;
const MAX_MILESTONES: usize = 256;
const MAX_SOURCE_EXCERPT_CHARS: usize = 360;
const REQUIRED_SECTIONS: [&str; 7] = [
    "Executive summary",
    "Supplier data",
    "Exceptions",
    "Milestone risks",
    "Current evidence",
    "Sources",
    "Next actions",
];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ComposeEvidenceReportRequest {
    supplier_analysis: SupplierAnalysis,
    milestone_analysis: MilestoneAnalysis,
    official_page_receipts: Vec<OfficialPageReceipt>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SupplierAnalysis {
    source_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    audit_year: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    quarter: Option<String>,
    supplier_count: usize,
    exception_count: usize,
    has_exception: bool,
    suppliers: Vec<SupplierRecord>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SupplierRecord {
    name: String,
    historical_settled_rate: f64,
    active_quote: f64,
    variance: f64,
    exceeds_historical: bool,
    status: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MilestoneAnalysis {
    source_sha256: String,
    milestone_count: usize,
    unfinished_count: usize,
    has_unfinished_milestones: bool,
    milestones: Vec<MilestoneRecord>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MilestoneRecord {
    milestone_id: String,
    name: String,
    target_date: String,
    status: String,
    owner: String,
    dependencies: Vec<String>,
    unfinished: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OfficialPageReceipt {
    requested_url: String,
    #[serde(default)]
    selected_url: String,
    #[serde(default)]
    attempted_urls: Vec<String>,
    #[serde(default)]
    fallback_used: bool,
    final_url: String,
    accessed_at_utc: String,
    status_code: u16,
    content_type: String,
    content: String,
    content_sha256: String,
    content_bytes: usize,
    content_truncated: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ComposedEvidenceReport {
    content: String,
    content_sha256: String,
    byte_count: usize,
    supplier_analysis_sha256: String,
    milestone_analysis_sha256: String,
    official_evidence_sha256: String,
    source_count: usize,
    required_sections: Vec<String>,
    composition_method: &'static str,
}

pub(crate) fn register_task_tool() -> Result<(), String> {
    crate::tools::task_tool_runtime::register(TaskToolRegistration {
        operation: OPERATION,
        validate: validate_registration,
        validate_resolved: validate_registration,
        resolve: crate::tools::task_tool_runtime::identity_resolver,
        execute: execute_registration,
        planner_context: None,
        schema: input_schema,
        metadata: TaskToolMetadata {
            description: "Compose one complete executive Markdown brief deterministically from exact typed supplier and milestone analyses plus verified official-page receipts.",
            risk_tier: TaskToolRiskTier::ReadOnly,
            approval_tier: TaskToolApprovalTier::Background,
            agent_error_code: "evidence_report_composition_failed",
            agent_error_boundary: "EvidenceReportComposition",
            execution_path: "The native compose_evidence_report tool validated the complete typed evidence set and rendered every supplier, every milestone, and every exact official-source receipt into bounded Markdown without model inference or invented claims.",
        },
    })
}

fn input_schema() -> Value {
    json!({
        "type":"object",
        "properties":{
            "supplierAnalysis":supplier_analysis_schema(),
            "milestoneAnalysis":milestone_analysis_schema(),
            "officialPageReceipts":{
                "type":"array",
                "minItems":1,
                "maxItems":MAX_OFFICIAL_RECEIPTS,
                "items":official_page_receipt_schema()
            }
        },
        "required":["supplierAnalysis","milestoneAnalysis","officialPageReceipts"],
        "additionalProperties":false
    })
}

fn supplier_analysis_schema() -> Value {
    json!({
        "type":"object",
        "properties":{
            "sourceSha256":{"type":"string","minLength":64,"maxLength":64},
            "auditYear":{"type":"integer","minimum":2000,"maximum":2100},
            "quarter":{"type":"string","enum":["Q1","Q2","Q3","Q4"]},
            "supplierCount":{"type":"integer","minimum":1,"maximum":MAX_SUPPLIERS},
            "exceptionCount":{"type":"integer","minimum":0,"maximum":MAX_SUPPLIERS},
            "hasException":{"type":"boolean"},
            "suppliers":{
                "type":"array",
                "minItems":1,
                "maxItems":MAX_SUPPLIERS,
                "items":{
                    "type":"object",
                    "properties":{
                        "name":{"type":"string","minLength":1,"maxLength":256},
                        "historicalSettledRate":{"type":"number","minimum":0},
                        "activeQuote":{"type":"number","minimum":0},
                        "variance":{"type":"number"},
                        "exceedsHistorical":{"type":"boolean"},
                        "status":{"type":"string","maxLength":256}
                    },
                    "required":["name","historicalSettledRate","activeQuote","variance","exceedsHistorical","status"],
                    "additionalProperties":false
                }
            }
        },
        "required":["sourceSha256","supplierCount","exceptionCount","hasException","suppliers"],
        "additionalProperties":false
    })
}

fn milestone_analysis_schema() -> Value {
    json!({
        "type":"object",
        "properties":{
            "sourceSha256":{"type":"string","minLength":64,"maxLength":64},
            "milestoneCount":{"type":"integer","minimum":1,"maximum":MAX_MILESTONES},
            "unfinishedCount":{"type":"integer","minimum":0,"maximum":MAX_MILESTONES},
            "hasUnfinishedMilestones":{"type":"boolean"},
            "milestones":{
                "type":"array",
                "minItems":1,
                "maxItems":MAX_MILESTONES,
                "items":{
                    "type":"object",
                    "properties":{
                        "milestoneId":{"type":"string","minLength":1,"maxLength":128},
                        "name":{"type":"string","minLength":1,"maxLength":512},
                        "targetDate":{"type":"string","minLength":10,"maxLength":10},
                        "status":{"type":"string","minLength":1,"maxLength":128},
                        "owner":{"type":"string","minLength":1,"maxLength":256},
                        "dependencies":{"type":"array","maxItems":MAX_MILESTONES,"items":{"type":"string","minLength":1,"maxLength":128}},
                        "unfinished":{"type":"boolean"}
                    },
                    "required":["milestoneId","name","targetDate","status","owner","dependencies","unfinished"],
                    "additionalProperties":false
                }
            }
        },
        "required":["sourceSha256","milestoneCount","unfinishedCount","hasUnfinishedMilestones","milestones"],
        "additionalProperties":false
    })
}

fn official_page_receipt_schema() -> Value {
    json!({
        "type":"object",
        "properties":{
            "requestedUrl":{"type":"string","minLength":8,"maxLength":8192},
            "selectedUrl":{"type":"string","minLength":8,"maxLength":8192},
            "attemptedUrls":{"type":"array","maxItems":3,"items":{"type":"string","minLength":8,"maxLength":8192}},
            "fallbackUsed":{"type":"boolean"},
            "finalUrl":{"type":"string","minLength":8,"maxLength":8192},
            "accessedAtUtc":{"type":"string","minLength":20,"maxLength":64},
            "statusCode":{"type":"integer","minimum":200,"maximum":299},
            "contentType":{"type":"string","minLength":1,"maxLength":256},
            "content":{"type":"string","minLength":1,"maxLength":MAX_REPORT_BYTES},
            "contentSha256":{"type":"string","minLength":64,"maxLength":64},
            "contentBytes":{"type":"integer","minimum":1,"maximum":MAX_REPORT_BYTES},
            "contentTruncated":{"type":"boolean"}
        },
        "required":["requestedUrl","finalUrl","accessedAtUtc","statusCode","contentType","content","contentSha256","contentBytes","contentTruncated"],
        "additionalProperties":false
    })
}

fn validate_registration(arguments: Value) -> Result<TaskToolValidation, String> {
    let request =
        serde_json::from_value::<ComposeEvidenceReportRequest>(arguments).map_err(|_| {
            "compose_evidence_report arguments do not match the registered schema.".to_string()
        })?;
    compose_report(&request)?;
    Ok(TaskToolValidation {
        arguments: serde_json::to_value(request).map_err(|error| error.to_string())?,
        potentially_effectful: false,
    })
}

fn execute_registration<'a>(
    context: TaskToolExecutionContext<'a>,
    arguments: Value,
) -> TaskToolFuture<'a> {
    Box::pin(async move {
        let request =
            serde_json::from_value::<ComposeEvidenceReportRequest>(arguments).map_err(|_| {
                "compose_evidence_report arguments do not match the registered schema.".to_string()
            })?;
        let execution_id = context
            .execution_id
            .ok_or_else(|| "Evidence report composition requires an active Task.".to_string())?;
        let task = require_agent_runtime_task(context.persistence, execution_id)?;
        let report = compose_report(&request)?;
        record_event(
            context.persistence,
            &task.task_run_id,
            "evidence_report.composed",
            EvidenceClass::VerifiedPostcondition,
            json!({
                "contentSha256":report.content_sha256,
                "byteCount":report.byte_count,
                "supplierAnalysisSha256":report.supplier_analysis_sha256,
                "milestoneAnalysisSha256":report.milestone_analysis_sha256,
                "officialEvidenceSha256":report.official_evidence_sha256,
                "sourceCount":report.source_count,
                "compositionMethod":report.composition_method,
            }),
        )?;
        Ok(ExecuteCommandResponse {
            operation: OPERATION.to_string(),
            status: CommandStatus::Completed,
            message: serde_json::to_string(&report).map_err(|error| error.to_string())?,
            metrics: None,
            claims: vec![format!(
                "CLAIM evidence_report_composed=true content_sha256={} byte_count={} supplier_count={} milestone_count={} source_count={}",
                report.content_sha256,
                report.byte_count,
                request.supplier_analysis.supplier_count,
                request.milestone_analysis.milestone_count,
                report.source_count,
            )],
            verified: true,
            model_used: None,
        })
    })
}

fn compose_report(
    request: &ComposeEvidenceReportRequest,
) -> Result<ComposedEvidenceReport, String> {
    validate_supplier_analysis(&request.supplier_analysis)?;
    validate_milestone_analysis(&request.milestone_analysis)?;
    validate_official_receipts(&request.official_page_receipts)?;

    let mut content = String::from("# Operations brief\n\n## Executive summary\n\n");
    content.push_str(&executive_summary(request));
    append_supplier_sections(&mut content, &request.supplier_analysis);

    content.push_str("\n## Milestone risks\n\n");
    content.push_str(
        "| ID | Milestone | Target date | Status | Owner | Dependencies | Unfinished |\n",
    );
    content.push_str("|---|---|---|---|---|---|---|\n");
    for milestone in &request.milestone_analysis.milestones {
        let dependencies = if milestone.dependencies.is_empty() {
            "None".to_string()
        } else {
            milestone
                .dependencies
                .iter()
                .map(|dependency| markdown_cell(dependency))
                .collect::<Vec<_>>()
                .join(", ")
        };
        content.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            markdown_cell(&milestone.milestone_id),
            markdown_cell(&milestone.name),
            milestone.target_date,
            markdown_cell(&milestone.status),
            markdown_cell(&milestone.owner),
            dependencies,
            if milestone.unfinished { "Yes" } else { "No" },
        ));
    }
    if request.milestone_analysis.unfinished_count == 0 {
        content.push_str("\nNo milestone in the typed Project evidence is unfinished.\n");
    } else {
        content.push_str(&format!(
            "\n{} of {} milestones are unfinished. Completed milestones remain in the table so the dependency chain is complete.\n",
            request.milestone_analysis.unfinished_count,
            request.milestone_analysis.milestone_count,
        ));
    }

    content.push_str("\n## Current evidence\n\n");
    content.push_str(&format!(
        "{} official source receipts were fetched successfully and integrity-bound to this brief. Each bounded excerpt below is whitespace-normalized retrieved source text, not an inferred trend. The receipts do not provide a typed, fixture-date-aligned comparison series, so this brief makes no unsupported claim that fuel, freight, supplier, or milestone conditions changed since the local fixture dates.\n\n",
        request.official_page_receipts.len(),
    ));
    for (index, receipt) in request.official_page_receipts.iter().enumerate() {
        content.push_str(&format!(
            "**Source {}:** <{}> — accessed {}\n\n> Retrieved source text: {}\n\n",
            index + 1,
            receipt.final_url,
            receipt.accessed_at_utc,
            normalized_excerpt(&receipt.content),
        ));
    }

    content.push_str("\n## Sources\n\n");
    for (index, receipt) in request.official_page_receipts.iter().enumerate() {
        content.push_str(&format!(
            "{}. <{}> — accessed {} UTC receipt; HTTP {}; content SHA-256 `{}`; {} bytes; content truncated: {}.\n",
            index + 1,
            receipt.final_url,
            receipt.accessed_at_utc,
            receipt.status_code,
            receipt.content_sha256,
            receipt.content_bytes,
            if receipt.content_truncated { "yes" } else { "no" },
        ));
    }

    content.push_str("\n## Next actions\n\n");
    if request.supplier_analysis.has_exception {
        content
            .push_str("- Reconcile each supplier exception shown above before committing spend.\n");
    } else {
        content.push_str("- Preserve the current supplier reconciliation record.\n");
    }
    if request.milestone_analysis.has_unfinished_milestones {
        content.push_str("- Review unfinished milestone owners, target dates, and dependency order shown above.\n");
    } else {
        content.push_str("- Preserve the completed milestone record.\n");
    }
    content.push_str("- Review the cited official pages before making any market-trend decision; this deterministic brief intentionally does not infer untyped web claims.\n");

    if content.len() > MAX_REPORT_BYTES {
        return Err("The composed evidence report exceeds the bounded report size.".to_string());
    }
    let supplier_analysis_sha256 = digest_json(&request.supplier_analysis)?;
    let milestone_analysis_sha256 = digest_json(&request.milestone_analysis)?;
    let official_evidence_sha256 = digest_json(&request.official_page_receipts)?;
    Ok(ComposedEvidenceReport {
        content_sha256: sha256_hex(content.as_bytes()),
        byte_count: content.len(),
        source_count: request.official_page_receipts.len(),
        required_sections: REQUIRED_SECTIONS
            .iter()
            .map(|section| (*section).to_string())
            .collect(),
        composition_method: "deterministic_typed_evidence_v1",
        supplier_analysis_sha256,
        milestone_analysis_sha256,
        official_evidence_sha256,
        content,
    })
}

fn append_supplier_sections(content: &mut String, analysis: &SupplierAnalysis) {
    content.push_str("\n\n## Supplier data\n\n");
    content.push_str(
        "| Supplier | Historical settled rate | Active quote | Variance | Status | Exception |\n",
    );
    content.push_str("|---|---:|---:|---:|---|---|\n");
    for supplier in &analysis.suppliers {
        content.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            markdown_cell(&supplier.name),
            currency(supplier.historical_settled_rate, false),
            currency(supplier.active_quote, false),
            currency(supplier.variance, true),
            markdown_cell(&supplier.status),
            if supplier.exceeds_historical {
                "Yes"
            } else {
                "No"
            },
        ));
    }

    content.push_str("\n## Exceptions\n\n");
    let exceptions = analysis
        .suppliers
        .iter()
        .filter(|supplier| supplier.exceeds_historical)
        .collect::<Vec<_>>();
    if exceptions.is_empty() {
        content.push_str("No supplier active quote exceeds its historical settled rate.\n");
        return;
    }
    for supplier in exceptions {
        content.push_str(&format!(
            "- {} is marked {}: its active quote of {} exceeds its historical settled rate of {} by {}.\n",
            markdown_inline(&supplier.name),
            markdown_inline(&supplier.status),
            currency(supplier.active_quote, false),
            currency(supplier.historical_settled_rate, false),
            currency(supplier.variance, true),
        ));
    }
}

fn executive_summary(request: &ComposeEvidenceReportRequest) -> String {
    let audit_period = match (
        request.supplier_analysis.quarter.as_deref(),
        request.supplier_analysis.audit_year,
    ) {
        (Some(quarter), Some(year)) => format!("{quarter} {year}"),
        (Some(quarter), None) => quarter.to_string(),
        (None, Some(year)) => year.to_string(),
        (None, None) => "the supplied audit period".to_string(),
    };
    format!(
        "For {audit_period}, the typed local evidence covers {} suppliers with {} exception{} and {} Project milestones with {} unfinished. {} official public sources were retrieved with exact URL and UTC access receipts. Supplier and milestone findings below come only from the typed local analyses; the source section reports only verified retrieval facts, and no untyped web trend is asserted.",
        request.supplier_analysis.supplier_count,
        request.supplier_analysis.exception_count,
        if request.supplier_analysis.exception_count == 1 { "" } else { "s" },
        request.milestone_analysis.milestone_count,
        request.milestone_analysis.unfinished_count,
        request.official_page_receipts.len(),
    )
}

fn validate_supplier_analysis(analysis: &SupplierAnalysis) -> Result<(), String> {
    if !valid_sha256(&analysis.source_sha256)
        || analysis.suppliers.is_empty()
        || analysis.suppliers.len() > MAX_SUPPLIERS
        || analysis.supplier_count != analysis.suppliers.len()
        || analysis
            .audit_year
            .is_some_and(|year| !(2000..=2100).contains(&year))
        || analysis
            .quarter
            .as_deref()
            .is_some_and(|quarter| !matches!(quarter, "Q1" | "Q2" | "Q3" | "Q4"))
    {
        return Err("The typed supplier analysis is internally inconsistent.".to_string());
    }
    let mut names = HashSet::new();
    let mut exception_count = 0usize;
    for supplier in &analysis.suppliers {
        let variance = supplier.active_quote - supplier.historical_settled_rate;
        if supplier.name.trim().is_empty()
            || supplier.name.len() > 256
            || supplier.status.len() > 256
            || !supplier.historical_settled_rate.is_finite()
            || !supplier.active_quote.is_finite()
            || !supplier.variance.is_finite()
            || supplier.historical_settled_rate < 0.0
            || supplier.active_quote < 0.0
            || !approximately_equal(supplier.variance, variance)
            || supplier.exceeds_historical != (variance > 0.0)
            || !names.insert(supplier.name.trim().to_ascii_lowercase())
        {
            return Err("The typed supplier analysis contains an invalid record.".to_string());
        }
        exception_count += usize::from(supplier.exceeds_historical);
    }
    if analysis.exception_count != exception_count
        || analysis.has_exception != (exception_count > 0)
    {
        return Err("The typed supplier exception summary is internally inconsistent.".to_string());
    }
    Ok(())
}

fn validate_milestone_analysis(analysis: &MilestoneAnalysis) -> Result<(), String> {
    if !valid_sha256(&analysis.source_sha256)
        || analysis.milestones.is_empty()
        || analysis.milestones.len() > MAX_MILESTONES
        || analysis.milestone_count != analysis.milestones.len()
    {
        return Err("The typed milestone analysis is internally inconsistent.".to_string());
    }
    let mut ids = HashSet::new();
    let mut unfinished_count = 0usize;
    for milestone in &analysis.milestones {
        if milestone.milestone_id.trim().is_empty()
            || milestone.milestone_id.len() > 128
            || milestone.name.trim().is_empty()
            || milestone.name.len() > 512
            || milestone.status.trim().is_empty()
            || milestone.status.len() > 128
            || milestone.owner.trim().is_empty()
            || milestone.owner.len() > 256
            || NaiveDate::parse_from_str(&milestone.target_date, "%Y-%m-%d").is_err()
            || milestone.unfinished != !milestone.status.eq_ignore_ascii_case("COMPLETED")
            || !ids.insert(milestone.milestone_id.trim().to_string())
        {
            return Err("The typed milestone analysis contains an invalid record.".to_string());
        }
        let mut dependencies = HashSet::new();
        if milestone.dependencies.iter().any(|dependency| {
            dependency.trim().is_empty()
                || dependency.len() > 128
                || !dependencies.insert(dependency.trim().to_string())
        }) {
            return Err("The typed milestone analysis contains an invalid dependency.".to_string());
        }
        unfinished_count += usize::from(milestone.unfinished);
    }
    if analysis.milestones.iter().any(|milestone| {
        milestone
            .dependencies
            .iter()
            .any(|dependency| !ids.contains(dependency.trim()))
    }) {
        return Err("The typed milestone analysis names an unknown dependency.".to_string());
    }
    if analysis.unfinished_count != unfinished_count
        || analysis.has_unfinished_milestones != (unfinished_count > 0)
    {
        return Err("The typed milestone summary is internally inconsistent.".to_string());
    }
    Ok(())
}

fn validate_official_receipts(receipts: &[OfficialPageReceipt]) -> Result<(), String> {
    if receipts.is_empty() || receipts.len() > MAX_OFFICIAL_RECEIPTS {
        return Err("The report requires a bounded official-source receipt set.".to_string());
    }
    let mut final_urls = HashSet::new();
    for receipt in receipts {
        let selected_url = if receipt.selected_url.trim().is_empty() {
            receipt.requested_url.as_str()
        } else {
            receipt.selected_url.as_str()
        };
        let attempted_urls = if receipt.attempted_urls.is_empty() {
            vec![receipt.requested_url.as_str()]
        } else {
            receipt
                .attempted_urls
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        };
        let mut unique_attempts = HashSet::new();
        if valid_public_https_url(&receipt.requested_url).is_none()
            || valid_public_https_url(selected_url).is_none()
            || attempted_urls.len() > 3
            || attempted_urls
                .iter()
                .any(|url| valid_public_https_url(url).is_none() || !unique_attempts.insert(*url))
            || attempted_urls.first().copied() != Some(receipt.requested_url.as_str())
            || attempted_urls.last().copied() != Some(selected_url)
            || receipt.fallback_used != (selected_url != receipt.requested_url)
            || valid_public_https_url(&receipt.final_url).is_none()
            || DateTime::parse_from_rfc3339(&receipt.accessed_at_utc)
                .ok()
                .is_none_or(|value| value.offset().local_minus_utc() != 0)
            || !(200..=299).contains(&receipt.status_code)
            || receipt.content_type.trim().is_empty()
            || receipt.content_type.len() > 256
            || receipt.content.trim().is_empty()
            || receipt.content.len() > MAX_REPORT_BYTES
            || receipt.content_bytes != receipt.content.len()
            || !valid_sha256(&receipt.content_sha256)
            || receipt.content_sha256 != sha256_hex(receipt.content.as_bytes())
            || !final_urls.insert(receipt.final_url.clone())
        {
            return Err(
                "An official-page receipt is invalid or internally inconsistent.".to_string(),
            );
        }
    }
    Ok(())
}

fn markdown_cell(value: &str) -> String {
    markdown_inline(value)
}

fn markdown_inline(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    let mut prior_space = false;
    for character in value.trim().chars() {
        if character.is_whitespace() {
            if !prior_space {
                escaped.push(' ');
                prior_space = true;
            }
            continue;
        }
        prior_space = false;
        if matches!(
            character,
            '\\' | '`' | '*' | '_' | '{' | '}' | '[' | ']' | '<' | '>' | '|' | '#'
        ) {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

fn currency(value: f64, explicit_positive: bool) -> String {
    let sign = if value < 0.0 {
        "-"
    } else if explicit_positive && value > 0.0 {
        "+"
    } else {
        ""
    };
    let absolute = value.abs().to_string();
    let (integer, fraction) = absolute
        .split_once('.')
        .map_or((absolute.as_str(), None), |(integer, fraction)| {
            (integer, Some(fraction))
        });
    let mut grouped = String::with_capacity(integer.len() + integer.len() / 3);
    for (index, character) in integer.chars().enumerate() {
        if index > 0 && (integer.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(character);
    }
    if let Some(fraction) = fraction {
        grouped.push('.');
        grouped.push_str(fraction);
    }
    format!("{sign}${grouped}")
}

fn normalized_excerpt(content: &str) -> String {
    let mut excerpt = String::new();
    for word in content.split_whitespace() {
        let separator = usize::from(!excerpt.is_empty());
        if excerpt
            .chars()
            .count()
            .saturating_add(separator)
            .saturating_add(word.chars().count())
            > MAX_SOURCE_EXCERPT_CHARS
        {
            break;
        }
        if separator == 1 {
            excerpt.push(' ');
        }
        excerpt.push_str(word);
    }
    if excerpt.is_empty() {
        content
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .chars()
            .take(MAX_SOURCE_EXCERPT_CHARS)
            .collect()
    } else {
        excerpt
    }
}

fn approximately_equal(left: f64, right: f64) -> bool {
    let scale = left.abs().max(right.abs()).max(1.0);
    (left - right).abs() <= scale * 1e-9
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_public_https_url(value: &str) -> Option<Url> {
    let url = Url::parse(value).ok()?;
    (url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && value.len() <= 8_192)
        .then_some(url)
}

fn digest_json(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_vec(value)
        .map(|bytes| sha256_hex(&bytes))
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> ComposeEvidenceReportRequest {
        let source_a = "Current official fuel conditions".to_string();
        let source_b = "Current official freight conditions".to_string();
        ComposeEvidenceReportRequest {
            supplier_analysis: SupplierAnalysis {
                source_sha256: "a".repeat(64),
                audit_year: Some(2026),
                quarter: Some("Q2".to_string()),
                supplier_count: 2,
                exception_count: 1,
                has_exception: true,
                suppliers: vec![
                    SupplierRecord {
                        name: "Apex Cargo".to_string(),
                        historical_settled_rate: 45_000.0,
                        active_quote: 46_500.0,
                        variance: 1_500.0,
                        exceeds_historical: true,
                        status: "PENDING_RECONCILIATION".to_string(),
                    },
                    SupplierRecord {
                        name: "Vanguard Freight".to_string(),
                        historical_settled_rate: 31_000.0,
                        active_quote: 31_000.0,
                        variance: 0.0,
                        exceeds_historical: false,
                        status: "ALIGNED".to_string(),
                    },
                ],
            },
            milestone_analysis: MilestoneAnalysis {
                source_sha256: "b".repeat(64),
                milestone_count: 3,
                unfinished_count: 2,
                has_unfinished_milestones: true,
                milestones: vec![
                    MilestoneRecord {
                        milestone_id: "M1".to_string(),
                        name: "Security isolation".to_string(),
                        target_date: "2026-07-06".to_string(),
                        status: "COMPLETED".to_string(),
                        owner: "Alex".to_string(),
                        dependencies: vec![],
                        unfinished: false,
                    },
                    MilestoneRecord {
                        milestone_id: "M2".to_string(),
                        name: "Localization integration".to_string(),
                        target_date: "2026-07-10".to_string(),
                        status: "IN_PROGRESS".to_string(),
                        owner: "Alex".to_string(),
                        dependencies: vec!["M1".to_string()],
                        unfinished: true,
                    },
                    MilestoneRecord {
                        milestone_id: "M3".to_string(),
                        name: "Release validation".to_string(),
                        target_date: "2026-07-15".to_string(),
                        status: "PENDING".to_string(),
                        owner: "OOMU".to_string(),
                        dependencies: vec!["M2".to_string()],
                        unfinished: true,
                    },
                ],
            },
            official_page_receipts: vec![
                official_receipt(
                    "https://www.eia.gov/petroleum/gasdiesel/",
                    "2026-07-22T15:00:00Z",
                    source_a,
                ),
                official_receipt(
                    "https://ops.fhwa.dot.gov/freight/",
                    "2026-07-22T15:01:00Z",
                    source_b,
                ),
            ],
        }
    }

    fn official_receipt(url: &str, accessed_at_utc: &str, content: String) -> OfficialPageReceipt {
        OfficialPageReceipt {
            requested_url: url.to_string(),
            selected_url: url.to_string(),
            attempted_urls: vec![url.to_string()],
            fallback_used: false,
            final_url: url.to_string(),
            accessed_at_utc: accessed_at_utc.to_string(),
            status_code: 200,
            content_type: "text/html".to_string(),
            content_sha256: sha256_hex(content.as_bytes()),
            content_bytes: content.len(),
            content,
            content_truncated: false,
        }
    }

    #[test]
    fn deterministic_brief_contains_every_typed_record_and_exact_source_receipt() {
        let request = request();
        let report = compose_report(&request).expect("deterministic report");
        for section in REQUIRED_SECTIONS {
            assert!(report.content.contains(&format!("## {section}")));
        }
        for supplier in &request.supplier_analysis.suppliers {
            assert!(report.content.contains(&supplier.name));
            assert!(report
                .content
                .contains(&currency(supplier.historical_settled_rate, false)));
            assert!(report
                .content
                .contains(&currency(supplier.active_quote, false)));
            assert!(report.content.contains(&currency(supplier.variance, true)));
            assert!(report
                .content
                .contains(&supplier.status.replace('_', "\\_")));
        }
        for milestone in &request.milestone_analysis.milestones {
            assert!(report.content.contains(&milestone.milestone_id));
            assert!(report.content.contains(&milestone.name));
            assert!(report.content.contains(&milestone.target_date));
            assert!(report
                .content
                .contains(&milestone.status.replace('_', "\\_")));
            assert!(report.content.contains(&milestone.owner));
            for dependency in &milestone.dependencies {
                assert!(report.content.contains(dependency));
            }
        }
        for receipt in &request.official_page_receipts {
            assert!(report.content.contains(&receipt.final_url));
            assert!(report.content.contains(&receipt.accessed_at_utc));
        }
        assert!(report.content.contains("M1 | Security isolation"));
        assert!(report.content.contains("no unsupported claim"));
        assert!(report.content.contains("+$1,500"));
        assert!(report.content.contains("$0"));
        assert!(report
            .content
            .contains("Retrieved source text: Current official fuel conditions"));
        assert_eq!(report.content_sha256, sha256_hex(report.content.as_bytes()));
        assert_eq!(report.byte_count, report.content.len());
        assert_eq!(report.composition_method, "deterministic_typed_evidence_v1");
    }

    #[test]
    fn deterministic_brief_crosses_the_production_evidence_validator_unchanged() {
        let request = request();
        let report = compose_report(&request).expect("deterministic report");
        let _ = crate::tools::evidence_report_validation::register_task_tool();

        let validation = crate::tools::task_tool_runtime::validate_if_registered(
            "validate_evidence_report",
            json!({
                "content": report.content,
                "supplierAnalysis": request.supplier_analysis,
                "milestoneAnalysis": request.milestone_analysis,
                "officialPageReceipts": request.official_page_receipts,
                "requiredSections": REQUIRED_SECTIONS,
            }),
        )
        .expect("validator registration")
        .expect("composed bytes satisfy the strict evidence validator");

        assert!(!validation.potentially_effectful);
    }

    #[test]
    fn malformed_typed_inputs_and_receipts_are_rejected_before_composition() {
        let mut malformed = request();
        malformed.milestone_analysis.milestones[0].unfinished = true;
        assert!(compose_report(&malformed).is_err());

        let mut malformed = request();
        malformed.supplier_analysis.suppliers[0].variance = 1.0;
        assert!(compose_report(&malformed).is_err());

        let mut malformed = request();
        malformed.official_page_receipts[0].content_bytes += 1;
        assert!(compose_report(&malformed).is_err());
    }

    #[test]
    fn schema_is_closed_and_tool_is_read_only() {
        let schema = input_schema();
        assert_eq!(schema["additionalProperties"], json!(false));
        assert_eq!(
            schema["required"],
            json!([
                "supplierAnalysis",
                "milestoneAnalysis",
                "officialPageReceipts"
            ])
        );
        let validation =
            validate_registration(serde_json::to_value(request()).expect("request serialization"))
                .expect("valid registered input");
        assert!(!validation.potentially_effectful);
    }
}
