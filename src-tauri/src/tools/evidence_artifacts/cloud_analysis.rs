use super::{Milestone, COMPARISON_OPERATION, RECOVERY_OPERATION};
use crate::{
    agent_manager::AgentManager,
    inference::{InferenceMessage, InferenceRequest},
    p0_contracts::EvidenceClass,
    tools::{task_runtime::record_event, task_tool_runtime::TaskToolExecutionContext},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use tauri::Manager;

const SPECIALIST_INITIAL_OUTPUT_TOKENS: u32 = 4_096;
const SPECIALIST_RETRY_OUTPUT_TOKENS: u32 = 8_192;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ComparisonEmphasis {
    ExecutionBoundary,
    SchedulingAuthority,
    Auditability,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ComparisonImplication {
    SeparateScheduleAndLedger,
    SurfaceLocalAndRemote,
    PreserveApprovalReceipts,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ComparisonAnalysis {
    pub executive_emphasis: ComparisonEmphasis,
    pub ordered_implication_ids: Vec<ComparisonImplication>,
}

pub(super) struct VerifiedComparisonAnalysis(ComparisonAnalysis);

impl VerifiedComparisonAnalysis {
    pub(super) fn get(&self) -> &ComparisonAnalysis {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RecoveryRisk {
    PrerequisiteSlip,
    SecurityValidationFailure,
    OwnerCapacityBlock,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RecoveryExecutionMode {
    ParallelAcrossOwnersSerialWithinOwner,
    SerialOnly,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RecoveryAnalysis {
    pub release_milestone_id: String,
    pub unfinished_milestone_ids: Vec<String>,
    pub execution_mode: RecoveryExecutionMode,
    pub ordered_risk_ids: Vec<RecoveryRisk>,
}

pub(super) struct VerifiedRecoveryAnalysis(RecoveryAnalysis);

impl VerifiedRecoveryAnalysis {
    pub(super) fn get(&self) -> &RecoveryAnalysis {
        &self.0
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CloudAnalysisReceipt {
    provider_config_id: String,
    provider_id: String,
    model_id: String,
    response_id: Option<String>,
    response_sha256: String,
    evidence_sha256: String,
    latency_ms: u128,
}

pub(super) async fn comparison(
    context: &TaskToolExecutionContext<'_>,
    task_run_id: &str,
) -> Result<VerifiedComparisonAnalysis, String> {
    let evidence = json!({
        "verifiedFacts": [
            {"id":"openclaw_scheduler","fact":"OpenClaw Cron is its precise scheduler."},
            {"id":"openclaw_ledger","fact":"OpenClaw tracks detached work in a background-task ledger; task records are not schedulers."},
            {"id":"openclaw_context","fact":"OpenClaw scheduled work may use an isolated or shared session."},
            {"id":"cowork_schedule","fact":"Cowork scheduled tasks run recurringly or on demand, each in its own session."},
            {"id":"cowork_capabilities","fact":"Cowork scheduled tasks can use connected tools, skills, plugins, and web research."},
            {"id":"cowork_boundary","fact":"Cowork remote tasks cannot use a computer folder; local files or apps require local execution."}
        ],
        "allowedImplicationIds": [
            "separate_schedule_and_ledger",
            "surface_local_and_remote",
            "preserve_approval_receipts"
        ]
    });
    let system = "You are OOMU's cloud comparison specialist. Analyze only the supplied native-verified facts. Return exactly one JSON object with executiveEmphasis (execution_boundary, scheduling_authority, or auditability) and orderedImplicationIds containing each allowed implication ID exactly once. Do not add prose, markdown, claims, or fields.";
    let (analysis, receipt) = invoke::<ComparisonAnalysis>(context, system, evidence).await?;
    validate_comparison(&analysis)?;
    record(context, task_run_id, COMPARISON_OPERATION, &receipt)?;
    Ok(VerifiedComparisonAnalysis(analysis))
}

pub(super) async fn recovery(
    context: &TaskToolExecutionContext<'_>,
    task_run_id: &str,
    milestones: &[Milestone],
) -> Result<VerifiedRecoveryAnalysis, String> {
    let evidence = json!({
        "milestones": milestones,
        "hardConstraints": {
            "oneOwnerCapacity": true,
            "businessHoursOnly": true,
            "contingencyReservePercent": 20,
            "securityValidationBeforeReleaseValidation": true
        },
        "allowedRiskIds": [
            "prerequisite_slip",
            "security_validation_failure",
            "owner_capacity_block"
        ]
    });
    let system = "You are OOMU's cloud recovery-planning specialist. Analyze only the supplied milestone records and hard constraints. Return exactly one JSON object with releaseMilestoneId, unfinishedMilestoneIds, executionMode (parallel_across_owners_serial_within_owner or serial_only), and orderedRiskIds containing each allowed risk ID exactly once. Preserve exact milestone IDs. Do not add prose, markdown, dates, durations, claims, or fields.";
    let (analysis, receipt) = invoke::<RecoveryAnalysis>(context, system, evidence).await?;
    validate_risks(&analysis.ordered_risk_ids)?;
    record(context, task_run_id, RECOVERY_OPERATION, &receipt)?;
    Ok(VerifiedRecoveryAnalysis(analysis))
}

#[cfg(test)]
pub(super) fn verified_comparison_for_test(
    analysis: ComparisonAnalysis,
) -> VerifiedComparisonAnalysis {
    validate_comparison(&analysis).expect("valid comparison fixture");
    VerifiedComparisonAnalysis(analysis)
}

#[cfg(test)]
pub(super) fn verified_recovery_for_test(analysis: RecoveryAnalysis) -> VerifiedRecoveryAnalysis {
    validate_risks(&analysis.ordered_risk_ids).expect("valid recovery fixture");
    VerifiedRecoveryAnalysis(analysis)
}

async fn invoke<T: for<'de> Deserialize<'de>>(
    context: &TaskToolExecutionContext<'_>,
    system_prompt: &str,
    evidence: Value,
) -> Result<(T, CloudAnalysisReceipt), String> {
    let route = context.model_route.ok_or_else(|| {
        "This specialist artifact has no signed model route. Review the plan and try again."
            .to_string()
    })?;
    if !route.selected_model.locality.eq_ignore_ascii_case("remote") {
        return Err("This specialist artifact requires the cloud model shown in its approved plan. No file was written.".to_string());
    }
    let provider_config_id = route.provider_config_id.as_deref().ok_or_else(|| {
        "The approved plan no longer identifies an exact cloud provider configuration. No file was written."
            .to_string()
    })?;
    let expected_provider_id = route.provider_id.as_deref().ok_or_else(|| {
        "The approved plan has no verifiable cloud provider identity. No file was written."
            .to_string()
    })?;
    let app = context.app.ok_or_else(|| {
        "The cloud specialist requires the OOMU app runtime. No file was written.".to_string()
    })?;
    let manager = app.state::<AgentManager>();
    let config = manager
        .select_provider_config(provider_config_id)
        .map_err(|_| {
            "OOMU could not reopen the approved cloud provider. No file was written.".to_string()
        })?
        .ok_or_else(|| {
            "The approved cloud provider is no longer configured. No file was written.".to_string()
        })?;
    let actual_provider_id = normalize_provider_id(&config.provider_id)?;
    if actual_provider_id != normalize_provider_id(expected_provider_id)? {
        return Err("The cloud provider changed after approval. Review the updated plan before OOMU writes anything.".to_string());
    }
    let model_id = route.selected_model.name.trim();
    if model_id.is_empty() {
        return Err("The approved cloud model is missing. No file was written.".to_string());
    }
    let evidence_text = serde_json::to_string(&evidence).map_err(|error| error.to_string())?;
    let evidence_sha256 = crate::foundation::digest::sha256_hex(evidence_text.as_bytes());
    let mut response = None;
    for output_tokens in [
        SPECIALIST_INITIAL_OUTPUT_TOKENS,
        SPECIALIST_RETRY_OUTPUT_TOKENS,
    ] {
        let candidate = crate::inference::run_provider_inference(specialist_inference_request(
            &config,
            &actual_provider_id,
            model_id,
            system_prompt,
            &evidence_text,
            output_tokens,
        ))
        .await
        .map_err(|error| {
            format!(
                "The approved cloud specialist could not complete its analysis. No file was written. {}",
                error.message
            )
        })?;
        if specialist_response_reached_token_limit(candidate.finish_reason.as_deref()) {
            if output_tokens == SPECIALIST_INITIAL_OUTPUT_TOKENS {
                continue;
            }
            return Err("The approved cloud specialist reached its output limit twice. No file was written; retry can continue from this step.".to_string());
        }
        response = Some(candidate);
        break;
    }
    let response = response.ok_or_else(|| {
        "The approved cloud specialist did not return a complete analysis. No file was written; retry can continue from this step."
            .to_string()
    })?;
    if normalize_provider_id(&response.provider_id)? != actual_provider_id
        || response.model_id.trim() != model_id
    {
        return Err("The cloud response did not come from the exact provider and model approved in the plan. No file was written.".to_string());
    }
    let response_text = strict_json_text(&response.text)?;
    let parsed = serde_json::from_str::<T>(response_text).map_err(|_| {
        "The approved cloud specialist returned an invalid analysis contract. No file was written."
            .to_string()
    })?;
    let receipt = CloudAnalysisReceipt {
        provider_config_id: provider_config_id.to_string(),
        provider_id: actual_provider_id,
        model_id: model_id.to_string(),
        response_id: response.response_id,
        response_sha256: crate::foundation::digest::sha256_hex(response_text.as_bytes()),
        evidence_sha256,
        latency_ms: response.latency_ms,
    };
    Ok((parsed, receipt))
}

fn specialist_inference_request(
    config: &crate::agent_manager::ConfiguredProvider,
    normalized_provider_id: &str,
    model_id: &str,
    system_prompt: &str,
    evidence_text: &str,
    max_tokens: u32,
) -> InferenceRequest {
    let minimal_reasoning = matches!(
        normalized_provider_id,
        "gemini" | "google" | "google_gemini" | "gemini_pro" | "gemini_flash"
    )
    .then(|| "minimal".to_string());
    InferenceRequest {
        provider_id: config.provider_id.clone(),
        model_id: model_id.to_string(),
        system_prompt: Some(system_prompt.to_string()),
        messages: vec![InferenceMessage {
            role: "user".to_string(),
            content: evidence_text.to_string(),
            attachments: Vec::new(),
        }],
        prompt: None,
        temperature: Some(0.0),
        max_tokens: Some(max_tokens),
        reasoning: minimal_reasoning,
        reasoning_budget_tokens: None,
        base_url: (!config.base_url.trim().is_empty()).then(|| config.base_url.clone()),
        api_key_label: (!config.api_key_label.trim().is_empty())
            .then(|| config.api_key_label.clone()),
        api_key: config.api_key.clone(),
    }
}

fn specialist_response_reached_token_limit(reason: Option<&str>) -> bool {
    reason.is_some_and(|reason| {
        matches!(
            reason.trim().to_ascii_lowercase().as_str(),
            "length"
                | "max_tokens"
                | "max_output_tokens"
                | "max_tokens_reached"
                | "max_tokens_exceeded"
                | "max_output_tokens_reached"
                | "token_limit"
                | "max_tokens_stop"
                | "max_tokens_limit"
        )
    })
}

fn strict_json_text(text: &str) -> Result<&str, String> {
    let mut trimmed = text.trim();
    if let Some(fenced) = trimmed
        .strip_prefix("```json")
        .and_then(|value| value.strip_suffix("```"))
    {
        trimmed = fenced.trim();
    }
    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return Err("The cloud specialist returned prose instead of its required analysis contract. No file was written.".to_string());
    }
    Ok(trimmed)
}

fn normalize_provider_id(value: &str) -> Result<String, String> {
    let normalized = value.trim().to_ascii_lowercase().replace('-', "_");
    if normalized.is_empty()
        || !normalized.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
    {
        return Err(
            "The approved cloud provider identity is invalid. No file was written.".to_string(),
        );
    }
    Ok(normalized)
}

fn validate_comparison(analysis: &ComparisonAnalysis) -> Result<(), String> {
    if analysis.ordered_implication_ids.len() != 3
        || analysis
            .ordered_implication_ids
            .iter()
            .copied()
            .collect::<HashSet<_>>()
            .len()
            != 3
    {
        return Err("The cloud specialist omitted or repeated a required comparison implication. No file was written.".to_string());
    }
    Ok(())
}

fn validate_risks(risks: &[RecoveryRisk]) -> Result<(), String> {
    if risks.len() != 3 || risks.iter().copied().collect::<HashSet<_>>().len() != 3 {
        return Err("The cloud specialist omitted or repeated a required recovery risk. No file was written.".to_string());
    }
    Ok(())
}

fn record(
    context: &TaskToolExecutionContext<'_>,
    task_run_id: &str,
    operation: &str,
    receipt: &CloudAnalysisReceipt,
) -> Result<(), String> {
    record_event(
        context.persistence,
        task_run_id,
        &format!("{operation}.cloud_analysis_verified"),
        EvidenceClass::ModelAssertion,
        serde_json::to_value(receipt).map_err(|error| error.to_string())?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specialist_contract_rejects_prose_and_safely_unwraps_one_json_fence() {
        assert!(strict_json_text("Here is the result: {\"ok\":true}").is_err());
        assert_eq!(
            strict_json_text("```json\n{\"ok\":true}\n```").unwrap(),
            "{\"ok\":true}"
        );
        assert_eq!(
            strict_json_text(" {\"ok\":true} ").unwrap(),
            "{\"ok\":true}"
        );
    }

    #[test]
    fn specialist_contract_rejects_duplicate_required_decisions() {
        let analysis = ComparisonAnalysis {
            executive_emphasis: ComparisonEmphasis::Auditability,
            ordered_implication_ids: vec![
                ComparisonImplication::PreserveApprovalReceipts,
                ComparisonImplication::PreserveApprovalReceipts,
                ComparisonImplication::SurfaceLocalAndRemote,
            ],
        };
        assert!(validate_comparison(&analysis).is_err());
    }

    #[test]
    fn gemini_specialist_uses_minimal_reasoning_and_a_complete_json_budget() {
        let config = crate::agent_manager::ConfiguredProvider {
            id: "provider-1".to_string(),
            provider_id: "gemini".to_string(),
            provider_name: "Google Gemini".to_string(),
            auth_method: "api_key".to_string(),
            base_url: String::new(),
            api_key_label: "test".to_string(),
            api_key: Some("secret".to_string()),
            credential_configured: true,
            custom_model_ids: "gemini-3.5-flash".to_string(),
            auto_route_target: true,
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        let request = specialist_inference_request(
            &config,
            "gemini",
            "gemini-3.5-flash",
            "Return JSON.",
            "{}",
            SPECIALIST_INITIAL_OUTPUT_TOKENS,
        );

        assert_eq!(request.max_tokens, Some(4_096));
        assert_eq!(request.reasoning.as_deref(), Some("minimal"));
        assert_eq!(request.messages[0].content, "{}");
        assert!(specialist_response_reached_token_limit(Some("MAX_TOKENS")));
        assert!(!specialist_response_reached_token_limit(Some("STOP")));
    }
}
