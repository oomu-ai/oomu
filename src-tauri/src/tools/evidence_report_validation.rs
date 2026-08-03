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
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::HashSet, sync::OnceLock};
use url::Url;

const OPERATION: &str = "validate_evidence_report";
const MAX_REPORT_BYTES: usize = 512 * 1024;
const MAX_REQUIRED_SECTIONS: usize = 32;
const MAX_OFFICIAL_RECEIPTS: usize = 16;
const MAX_SUPPLIERS: usize = 10_000;
const MAX_MILESTONES: usize = 256;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ValidateEvidenceReportRequest {
    content: String,
    supplier_analysis: SupplierAnalysis,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    milestone_analysis: Option<MilestoneAnalysis>,
    official_page_receipts: Vec<OfficialPageReceipt>,
    required_sections: Vec<String>,
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
struct VerifiedEvidenceReport {
    content: String,
    content_sha256: String,
    byte_count: usize,
    supplier_analysis_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    milestone_analysis_sha256: Option<String>,
    official_evidence_sha256: String,
    source_count: usize,
    required_sections: Vec<String>,
    verified: bool,
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
            description: "Verify that an evidence report contains every supplied typed business fact, official-source identity, access time, and required section before any artifact is written or delivered.",
            risk_tier: TaskToolRiskTier::ReadOnly,
            approval_tier: TaskToolApprovalTier::Background,
            agent_error_code: "evidence_report_validation_failed",
            agent_error_boundary: "EvidenceReportValidation",
            execution_path: "The native validate_evidence_report tool checked the exact Agent-authored bytes against typed analyses and official-page receipts, rejected unresolved or collapsed output, and returned a digest-bound read-only receipt.",
        },
    })
}

fn input_schema() -> Value {
    json!({
        "type":"object",
        "properties":{
            "content":{
                "type":"string",
                "minLength":1,
                "maxLength":MAX_REPORT_BYTES,
                "description":"Exact Markdown report content generated from the supplied evidence."
            },
            "supplierAnalysis":{
                "type":"object",
                "description":"Exact typed output from analyze_supplier_exceptions.",
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
            },
            "milestoneAnalysis":{
                "type":"object",
                "description":"Optional exact typed output from analyze_project_milestones.",
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
            },
            "officialPageReceipts":{
                "type":"array",
                "minItems":1,
                "maxItems":MAX_OFFICIAL_RECEIPTS,
                "description":"Exact verified outputs from fetch_official_page.",
                "items":{
                    "type":"object",
                    "properties":{
                        "requestedUrl":{"type":"string","minLength":8,"maxLength":8192},
                        "finalUrl":{"type":"string","minLength":8,"maxLength":8192},
                        "accessedAtUtc":{"type":"string","minLength":20,"maxLength":64},
                        "statusCode":{"type":"integer","minimum":200,"maximum":299},
                        "contentType":{"type":"string","minLength":1,"maxLength":256},
                        "content":{"type":"string","minLength":1,"maxLength":524288},
                        "contentSha256":{"type":"string","minLength":64,"maxLength":64},
                        "contentBytes":{"type":"integer","minimum":1,"maximum":524288},
                        "contentTruncated":{"type":"boolean"}
                    },
                    "required":["requestedUrl","finalUrl","accessedAtUtc","statusCode","contentType","content","contentSha256","contentBytes","contentTruncated"],
                    "additionalProperties":false
                }
            },
            "requiredSections":{
                "type":"array",
                "minItems":1,
                "maxItems":MAX_REQUIRED_SECTIONS,
                "description":"Exact human-facing section names that must appear as Markdown headings.",
                "items":{"type":"string","minLength":1,"maxLength":128}
            }
        },
        "required":["content","supplierAnalysis","officialPageReceipts","requiredSections"],
        "additionalProperties":false
    })
}

fn validate_registration(arguments: Value) -> Result<TaskToolValidation, String> {
    let request =
        serde_json::from_value::<ValidateEvidenceReportRequest>(arguments).map_err(|_| {
            "validate_evidence_report arguments do not match the registered schema.".to_string()
        })?;
    validate_report(&request)?;
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
            serde_json::from_value::<ValidateEvidenceReportRequest>(arguments).map_err(|_| {
                "validate_evidence_report arguments do not match the registered schema.".to_string()
            })?;
        let execution_id = context
            .execution_id
            .ok_or_else(|| "Evidence report validation requires an active Task.".to_string())?;
        let task = require_agent_runtime_task(context.persistence, execution_id)?;
        let receipt = validate_report(&request)?;
        record_event(
            context.persistence,
            &task.task_run_id,
            "evidence_report.validated",
            EvidenceClass::VerifiedPostcondition,
            json!({
                "contentSha256":receipt.content_sha256,
                "byteCount":receipt.byte_count,
                "supplierAnalysisSha256":receipt.supplier_analysis_sha256,
                "milestoneAnalysisSha256":receipt.milestone_analysis_sha256,
                "officialEvidenceSha256":receipt.official_evidence_sha256,
                "sourceCount":receipt.source_count,
                "requiredSections":receipt.required_sections,
            }),
        )?;
        Ok(ExecuteCommandResponse {
            operation: OPERATION.to_string(),
            status: CommandStatus::Completed,
            message: serde_json::to_string(&receipt).map_err(|error| error.to_string())?,
            metrics: None,
            claims: vec![format!(
                "CLAIM evidence_report_validated=true content_sha256={} byte_count={} source_count={}",
                receipt.content_sha256, receipt.byte_count, receipt.source_count
            )],
            verified: true,
            model_used: None,
        })
    })
}

fn validate_report(
    request: &ValidateEvidenceReportRequest,
) -> Result<VerifiedEvidenceReport, String> {
    validate_content(&request.content)?;
    validate_supplier_analysis(&request.supplier_analysis)?;
    if let Some(analysis) = &request.milestone_analysis {
        validate_milestone_analysis(analysis)?;
    }
    validate_official_receipts(&request.official_page_receipts)?;
    let required_sections = validate_required_sections(&request.required_sections)?;

    require_supplier_facts(&request.content, &request.supplier_analysis)?;
    if let Some(analysis) = &request.milestone_analysis {
        require_milestone_facts(&request.content, analysis)?;
    }
    require_official_evidence(&request.content, &request.official_page_receipts)?;
    require_sections(&request.content, &required_sections)?;

    let supplier_analysis_sha256 = digest_json(&request.supplier_analysis)?;
    let milestone_analysis_sha256 = request
        .milestone_analysis
        .as_ref()
        .map(digest_json)
        .transpose()?;
    let official_evidence_sha256 = digest_json(&request.official_page_receipts)?;
    Ok(VerifiedEvidenceReport {
        content: request.content.clone(),
        content_sha256: sha256_hex(request.content.as_bytes()),
        byte_count: request.content.len(),
        supplier_analysis_sha256,
        milestone_analysis_sha256,
        official_evidence_sha256,
        source_count: request.official_page_receipts.len(),
        required_sections,
        verified: true,
    })
}

fn validate_content(content: &str) -> Result<(), String> {
    if content.trim().is_empty() || content.len() > MAX_REPORT_BYTES {
        return Err("The evidence report is empty or exceeds the bounded report size.".to_string());
    }
    if contains_unresolved_placeholder(content) {
        return Err("The evidence report contains an unresolved placeholder.".to_string());
    }
    if has_repetition_collapse(content) {
        return Err("The evidence report contains repeated collapsed output.".to_string());
    }
    Ok(())
}

fn validate_supplier_analysis(analysis: &SupplierAnalysis) -> Result<(), String> {
    if !valid_sha256(&analysis.source_sha256)
        || analysis.suppliers.is_empty()
        || analysis.suppliers.len() > MAX_SUPPLIERS
        || analysis.supplier_count != analysis.suppliers.len()
        || analysis
            .audit_year
            .is_some_and(|year| !(2000..=2100).contains(&year))
        || analysis.quarter.as_deref().is_some_and(|quarter| {
            !matches!(
                quarter.trim().to_ascii_uppercase().as_str(),
                "Q1" | "Q2" | "Q3" | "Q4"
            )
        })
    {
        return Err("The typed supplier analysis is internally inconsistent.".to_string());
    }
    let mut names = HashSet::new();
    let mut exception_count = 0;
    for supplier in &analysis.suppliers {
        let expected_variance = supplier.active_quote - supplier.historical_settled_rate;
        if supplier.name.trim().is_empty()
            || supplier.name.len() > 256
            || supplier.status.len() > 256
            || !supplier.historical_settled_rate.is_finite()
            || !supplier.active_quote.is_finite()
            || !supplier.variance.is_finite()
            || supplier.historical_settled_rate < 0.0
            || supplier.active_quote < 0.0
            || !approximately_equal(supplier.variance, expected_variance)
            || supplier.exceeds_historical != (expected_variance > 0.0)
            || !names.insert(canonical_phrase(&supplier.name))
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
    }
    if analysis.milestones.iter().any(|milestone| {
        milestone
            .dependencies
            .iter()
            .any(|dependency| !ids.contains(dependency.trim()))
    }) {
        return Err("The typed milestone analysis names an unknown dependency.".to_string());
    }
    let unfinished_count = analysis
        .milestones
        .iter()
        .filter(|milestone| milestone.unfinished)
        .count();
    if analysis.unfinished_count != unfinished_count
        || analysis.has_unfinished_milestones != (unfinished_count > 0)
    {
        return Err("The typed milestone summary is internally inconsistent.".to_string());
    }
    Ok(())
}

fn validate_official_receipts(receipts: &[OfficialPageReceipt]) -> Result<(), String> {
    if receipts.is_empty() || receipts.len() > MAX_OFFICIAL_RECEIPTS {
        return Err(
            "The evidence report requires a bounded official-source receipt set.".to_string(),
        );
    }
    let mut final_urls = HashSet::new();
    for receipt in receipts {
        let requested = valid_public_https_url(&receipt.requested_url);
        let selected_url = if receipt.selected_url.trim().is_empty() {
            receipt.requested_url.as_str()
        } else {
            receipt.selected_url.as_str()
        };
        let selected = valid_public_https_url(selected_url);
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
        let attempts_are_valid = attempted_urls.len() <= 3
            && attempted_urls
                .iter()
                .all(|url| valid_public_https_url(url).is_some() && unique_attempts.insert(*url));
        let final_url = valid_public_https_url(&receipt.final_url);
        let accessed = DateTime::parse_from_rfc3339(&receipt.accessed_at_utc).ok();
        if requested.is_none()
            || selected.is_none()
            || !attempts_are_valid
            || attempted_urls.first().copied() != Some(receipt.requested_url.as_str())
            || attempted_urls.last().copied() != Some(selected_url)
            || receipt.fallback_used != (selected_url != receipt.requested_url)
            || final_url.is_none()
            || accessed.is_none_or(|value| value.offset().local_minus_utc() != 0)
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

fn validate_required_sections(sections: &[String]) -> Result<Vec<String>, String> {
    if sections.is_empty() || sections.len() > MAX_REQUIRED_SECTIONS {
        return Err("The evidence report requires a bounded non-empty section list.".to_string());
    }
    let mut normalized = HashSet::new();
    let mut validated = Vec::with_capacity(sections.len());
    for section in sections {
        let section = section.trim();
        let canonical = canonical_phrase(section);
        if section.is_empty()
            || section.len() > 128
            || section.contains(['\n', '\r', '#'])
            || canonical.is_empty()
            || !normalized.insert(canonical)
        {
            return Err(
                "The required report sections contain an invalid or duplicate name.".to_string(),
            );
        }
        validated.push(section.to_string());
    }
    Ok(validated)
}

fn require_supplier_facts(content: &str, analysis: &SupplierAnalysis) -> Result<(), String> {
    if analysis
        .audit_year
        .is_some_and(|year| !contains_number(content, year as f64))
        || analysis
            .quarter
            .as_deref()
            .is_some_and(|quarter| !contains_phrase(content, quarter))
    {
        return Err("The evidence report omits the supplier audit period.".to_string());
    }
    for supplier in &analysis.suppliers {
        let segments = record_segments(content, &[&supplier.name]);
        let present = segments.iter().any(|segment| {
            contains_phrase(segment, &supplier.name)
                && (supplier.status.trim().is_empty() || contains_phrase(segment, &supplier.status))
                && contains_number(segment, supplier.historical_settled_rate)
                && contains_number(segment, supplier.active_quote)
                && contains_number(segment, supplier.variance)
        });
        if !present {
            return Err(format!(
                "The evidence report omits one or more typed facts for supplier '{}'.",
                supplier.name
            ));
        }
    }
    Ok(())
}

fn require_milestone_facts(content: &str, analysis: &MilestoneAnalysis) -> Result<(), String> {
    for milestone in &analysis.milestones {
        let segments = record_segments(content, &[&milestone.milestone_id, &milestone.name]);
        let present = segments.iter().any(|segment| {
            contains_phrase(segment, &milestone.milestone_id)
                && contains_phrase(segment, &milestone.name)
                && segment.contains(&milestone.target_date)
                && contains_phrase(segment, &milestone.status)
                && contains_phrase(segment, &milestone.owner)
                && milestone
                    .dependencies
                    .iter()
                    .all(|dependency| contains_phrase(segment, dependency))
        });
        if !present {
            return Err(format!(
                "The evidence report omits one or more typed facts for milestone '{}'.",
                milestone.milestone_id
            ));
        }
    }
    Ok(())
}

fn require_official_evidence(
    content: &str,
    receipts: &[OfficialPageReceipt],
) -> Result<(), String> {
    if receipts.iter().any(|receipt| {
        !content.contains(&receipt.final_url) || !content.contains(&receipt.accessed_at_utc)
    }) {
        return Err(
            "The evidence report omits an exact official-source URL or UTC access time."
                .to_string(),
        );
    }
    Ok(())
}

fn require_sections(content: &str, sections: &[String]) -> Result<(), String> {
    let headings = markdown_headings(content);
    if sections.iter().any(|section| {
        let expected = canonical_phrase(section);
        !headings.iter().any(|heading| heading == &expected)
    }) {
        return Err("The evidence report omits a required Markdown section.".to_string());
    }
    Ok(())
}

fn markdown_headings(content: &str) -> Vec<String> {
    let lines = content.lines().collect::<Vec<_>>();
    let mut headings = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let hashes = trimmed.bytes().take_while(|byte| *byte == b'#').count();
        if (1..=6).contains(&hashes)
            && trimmed
                .as_bytes()
                .get(hashes)
                .is_some_and(u8::is_ascii_whitespace)
        {
            let heading = trimmed[hashes..].trim().trim_end_matches('#').trim();
            headings.push(canonical_phrase(heading));
        }
        if index > 0
            && trimmed.len() >= 3
            && (trimmed.bytes().all(|byte| byte == b'=')
                || trimmed.bytes().all(|byte| byte == b'-'))
        {
            let heading = canonical_phrase(lines[index - 1].trim());
            if !heading.is_empty() {
                headings.push(heading);
            }
        }
    }
    headings
}

fn record_segments(content: &str, identities: &[&str]) -> Vec<String> {
    let lines = content.lines().collect::<Vec<_>>();
    let mut segments = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if !identities
            .iter()
            .any(|identity| contains_phrase(line, identity))
        {
            continue;
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with('|') {
            segments.push((*line).to_string());
            continue;
        }
        let end = (index + 10).min(lines.len());
        let mut block = Vec::new();
        for candidate in &lines[index..end] {
            if !block.is_empty() && candidate.trim_start().starts_with('#') {
                break;
            }
            if !block.is_empty() && candidate.trim().is_empty() {
                break;
            }
            block.push(*candidate);
        }
        segments.push(block.join("\n"));
    }
    segments
}

fn contains_phrase(content: &str, phrase: &str) -> bool {
    let content = canonical_words(content);
    let phrase = canonical_words(phrase);
    !phrase.is_empty()
        && content
            .windows(phrase.len())
            .any(|candidate| candidate == phrase.as_slice())
}

fn canonical_phrase(value: &str) -> String {
    canonical_words(value).join(" ")
}

fn canonical_words(value: &str) -> Vec<String> {
    value
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(|word| word.to_lowercase())
        .collect()
}

fn contains_number(content: &str, expected: f64) -> bool {
    number_regex().find_iter(content).any(|matched| {
        let normalized = matched
            .as_str()
            .replace([',', ' ', '\u{00a0}', '\u{202f}'], "");
        normalized
            .parse::<f64>()
            .is_ok_and(|actual| approximately_equal(actual, expected))
    })
}

fn number_regex() -> &'static Regex {
    static NUMBER: OnceLock<Regex> = OnceLock::new();
    NUMBER.get_or_init(|| {
        Regex::new(r"[-+]?(?:\d{1,3}(?:[,\u{00a0}\u{202f} ]\d{3})+|\d+)(?:\.\d+)?")
            .expect("evidence report number regex is valid")
    })
}

fn approximately_equal(left: f64, right: f64) -> bool {
    let scale = left.abs().max(right.abs()).max(1.0);
    (left - right).abs() <= scale * 1e-9
}

fn contains_unresolved_placeholder(content: &str) -> bool {
    static PLACEHOLDER: OnceLock<Regex> = OnceLock::new();
    PLACEHOLDER
        .get_or_init(|| {
            Regex::new(
                r"(?ix)(?:\b(?:lorem\s+ipsum|todo|tbd|tbc)\b|\{\{[^}\n]+\}\}|<\s*YYYY(?:-[A-Z]{2})?[^>\n]*>|\[\s*(?:insert|placeholder|replace\s+me)[^\]\n]*\])",
            )
            .expect("evidence report placeholder regex is valid")
        })
        .is_match(content)
}

fn has_repetition_collapse(content: &str) -> bool {
    let mut seen_lines = std::collections::HashMap::new();
    for line in content.lines() {
        let canonical = canonical_phrase(line);
        if canonical.len() >= 40 {
            let count = seen_lines.entry(canonical).or_insert(0usize);
            *count += 1;
            if *count >= 3 {
                return true;
            }
        }
    }
    let words = canonical_words(content);
    for unit in 4..=24.min(words.len() / 3) {
        for start in 0..=words.len().saturating_sub(unit * 3) {
            if words[start..start + unit] == words[start + unit..start + unit * 2]
                && words[start..start + unit] == words[start + unit * 2..start + unit * 3]
            {
                return true;
            }
        }
    }
    false
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

    fn supplier_analysis() -> SupplierAnalysis {
        SupplierAnalysis {
            source_sha256: "a".repeat(64),
            audit_year: Some(2026),
            quarter: Some("Q2".to_string()),
            supplier_count: 2,
            exception_count: 1,
            has_exception: true,
            suppliers: vec![
                SupplierRecord {
                    name: "North Harbor Logistics".to_string(),
                    historical_settled_rate: 45_000.0,
                    active_quote: 46_500.0,
                    variance: 1_500.0,
                    exceeds_historical: true,
                    status: "PENDING_RECONCILIATION".to_string(),
                },
                SupplierRecord {
                    name: "Cedar Freight".to_string(),
                    historical_settled_rate: 31_000.0,
                    active_quote: 31_000.0,
                    variance: 0.0,
                    exceeds_historical: false,
                    status: "ALIGNED".to_string(),
                },
            ],
        }
    }

    fn milestone_analysis() -> MilestoneAnalysis {
        MilestoneAnalysis {
            source_sha256: "b".repeat(64),
            milestone_count: 2,
            unfinished_count: 1,
            has_unfinished_milestones: true,
            milestones: vec![
                MilestoneRecord {
                    milestone_id: "R1".to_string(),
                    name: "Security review".to_string(),
                    target_date: "2026-07-06".to_string(),
                    status: "COMPLETED".to_string(),
                    owner: "Morgan Lee".to_string(),
                    dependencies: Vec::new(),
                    unfinished: false,
                },
                MilestoneRecord {
                    milestone_id: "R2".to_string(),
                    name: "Release readiness".to_string(),
                    target_date: "2026-07-15".to_string(),
                    status: "IN_PROGRESS".to_string(),
                    owner: "Sam Rivera".to_string(),
                    dependencies: vec!["R1".to_string()],
                    unfinished: true,
                },
            ],
        }
    }

    fn official_receipts() -> Vec<OfficialPageReceipt> {
        [
            (
                "https://energy.example.gov/current",
                "2026-07-21T14:05:06.000Z",
                "Current energy conditions remain stable.",
            ),
            (
                "https://transport.example.gov/current",
                "2026-07-21T14:06:07.000Z",
                "Current transport conditions show limited delays.",
            ),
        ]
        .into_iter()
        .map(|(url, accessed_at_utc, content)| OfficialPageReceipt {
            requested_url: url.to_string(),
            selected_url: url.to_string(),
            attempted_urls: vec![url.to_string()],
            fallback_used: false,
            final_url: url.to_string(),
            accessed_at_utc: accessed_at_utc.to_string(),
            status_code: 200,
            content_type: "text/html".to_string(),
            content: content.to_string(),
            content_sha256: sha256_hex(content.as_bytes()),
            content_bytes: content.len(),
            content_truncated: false,
        })
        .collect()
    }

    fn valid_content() -> String {
        r#"# Executive summary

The 2026 Q2 review found one exception across two suppliers.

## Supplier variance

| Supplier | Historical settled rate | Active quote | Variance | Status |
|---|---:|---:|---:|---|
| North Harbor Logistics | $45,000 | $46,500 | $1,500 | Pending reconciliation |
| Cedar Freight | $31,000 | $31,000 | $0 | Aligned |

## Milestone risks

| ID | Milestone | Target date | Status | Owner | Dependencies |
|---|---|---|---|---|---|
| R1 | Security review | 2026-07-06 | Completed | Morgan Lee | None |
| R2 | Release readiness | 2026-07-15 | In progress | Sam Rivera | R1 |

## Current evidence

- https://energy.example.gov/current — accessed 2026-07-21T14:05:06.000Z
- https://transport.example.gov/current — accessed 2026-07-21T14:06:07.000Z

## Next actions

- [ ] Reconcile the open supplier exception.
"#
        .to_string()
    }

    fn request() -> ValidateEvidenceReportRequest {
        ValidateEvidenceReportRequest {
            content: valid_content(),
            supplier_analysis: supplier_analysis(),
            milestone_analysis: Some(milestone_analysis()),
            official_page_receipts: official_receipts(),
            required_sections: vec![
                "Executive summary".to_string(),
                "Supplier variance".to_string(),
                "Milestone risks".to_string(),
                "Current evidence".to_string(),
                "Next actions".to_string(),
            ],
        }
    }

    #[test]
    fn exact_typed_facts_and_source_receipts_produce_same_content_verified_receipt() {
        let request = request();
        let receipt = validate_report(&request).expect("report validates");
        assert_eq!(receipt.content, request.content);
        assert_eq!(receipt.byte_count, request.content.len());
        assert_eq!(
            receipt.content_sha256,
            sha256_hex(request.content.as_bytes())
        );
        assert_eq!(receipt.source_count, 2);
        assert!(receipt.milestone_analysis_sha256.is_some());
        assert!(receipt.verified);
    }

    #[test]
    fn omitted_supplier_or_milestone_business_fact_is_rejected() {
        let mut missing_supplier = request();
        missing_supplier.content = missing_supplier.content.replace("$46,500", "not stated");
        assert!(validate_report(&missing_supplier)
            .unwrap_err()
            .contains("North Harbor Logistics"));

        let mut missing_milestone = request();
        missing_milestone.content = missing_milestone
            .content
            .replace("Sam Rivera", "not stated");
        assert!(validate_report(&missing_milestone)
            .unwrap_err()
            .contains("R2"));
    }

    #[test]
    fn exact_final_url_timestamp_and_required_heading_are_enforced() {
        let mut missing_url = request();
        missing_url.content = missing_url
            .content
            .replace("https://transport.example.gov/current", "transport source");
        assert!(validate_report(&missing_url)
            .unwrap_err()
            .contains("official-source URL"));

        let mut missing_heading = request();
        missing_heading.content = missing_heading
            .content
            .replace("## Next actions", "Next actions");
        assert!(validate_report(&missing_heading)
            .unwrap_err()
            .contains("Markdown section"));
    }

    #[test]
    fn unresolved_or_repetition_collapsed_output_is_rejected_without_blocking_checklists() {
        let mut checklist = request();
        checklist
            .content
            .push_str("\n- [ ] Confirm the next review.\n");
        assert!(validate_report(&checklist).is_ok());

        let mut placeholder = request();
        placeholder.content.push_str("\nOwner: {{placeholder}}\n");
        assert!(validate_report(&placeholder)
            .unwrap_err()
            .contains("unresolved placeholder"));

        let mut repeated = request();
        repeated.content.push_str(
            "\nConditions remain unchanged without any additional verified evidence. Conditions remain unchanged without any additional verified evidence. Conditions remain unchanged without any additional verified evidence.\n",
        );
        assert!(validate_report(&repeated)
            .unwrap_err()
            .contains("repeated collapsed"));
    }

    #[test]
    fn registration_is_read_only_background_and_non_effectful() {
        let _ = register_task_tool();
        assert_eq!(
            crate::tools::task_tool_runtime::risk_tier(OPERATION),
            Ok(TaskToolRiskTier::ReadOnly)
        );
        assert_eq!(
            crate::tools::task_tool_runtime::approval_tier(OPERATION),
            Some(TaskToolApprovalTier::Background)
        );
        let validation = validate_registration(serde_json::to_value(request()).unwrap())
            .expect("registered request validates");
        assert!(!validation.potentially_effectful);
        assert!(input_schema()["required"]
            .as_array()
            .unwrap()
            .contains(&json!("officialPageReceipts")));
    }
}
