use super::*;

pub(super) async fn generate_cloud_plan_draft(
    objective: String,
    compiled_prompt: String,
    target: CloudPlannerTarget,
) -> Result<GeneratedActionPlanDraft, AgenticLoopError> {
    let first_response = run_cloud_planner_inference(
        &target,
        compiled_prompt.clone(),
        "You are OOMU's cloud action-plan compiler. Return only compact JSON matching the provided ActionPlan contract.",
    )
    .await;
    let response_text =
        first_response.map_err(|error| cloud_planner_inference_error(&target, "initial", error))?;
    let first_draft = generated_plan_from_text(objective.clone(), response_text);
    let Some(repair_reason) = cloud_plan_repair_reason(&objective, &first_draft) else {
        let mut draft = first_draft;
        draft.source = IntentSource::Cloud;
        return Ok(draft);
    };
    let repair_prompt = compile_cloud_planner_repair_prompt(
        &compiled_prompt,
        &repair_reason,
        &first_draft.generated_text,
    )?;
    let repaired_text = run_cloud_planner_inference(
        &target,
        repair_prompt,
        "You repair OOMU ActionPlans after strict schema and coverage validation. Return exactly one complete JSON ActionPlan and no prose.",
    )
    .await
    .map_err(|error| cloud_planner_inference_error(&target, "repair", error))?;
    let mut repaired = generated_plan_from_text(objective, repaired_text);
    if matches!(repaired.source, IntentSource::Degraded) {
        let reason = repaired
            .degraded_reason
            .clone()
            .unwrap_or_else(|| "ActionPlan JSON failed schema validation.".to_string());
        eprintln!(
            "CLOUD_PLANNER_OUTPUT_REJECTED phase=repair provider={} model={} reason={}",
            target.provider_id,
            target.model_id,
            compact_for_prompt(&reason, 240)
        );
        return Err(AgenticLoopError {
            code: "planner_output_unusable",
            boundary: "AgentPlanning",
            message: "The planning response did not match OOMU's required action format after one repair attempt. Your request is saved and no action was executed. Try again or choose another model."
                .to_string(),
            mlc_path: None,
        });
    } else {
        repaired.source = IntentSource::Cloud;
    }
    Ok(repaired)
}

async fn run_cloud_planner_inference(
    target: &CloudPlannerTarget,
    prompt: String,
    system_prompt: &str,
) -> Result<String, crate::inference::InferenceError> {
    let request = ProviderInferenceRequest {
        provider_id: target.provider_id.clone(),
        model_id: target.model_id.clone(),
        system_prompt: Some(system_prompt.to_string()),
        messages: vec![ProviderInferenceMessage {
            role: "user".to_string(),
            content: prompt,
            attachments: Vec::new(),
        }],
        prompt: None,
        temperature: Some(0.2),
        max_tokens: Some(8_192),
        reasoning: None,
        reasoning_budget_tokens: None,
        base_url: target.base_url.clone(),
        api_key_label: target.api_key_label.clone(),
        api_key: target.api_key.clone(),
    };
    crate::inference::run_provider_inference(request)
        .await
        .map(|response| response.text)
}

fn cloud_planner_inference_error(
    target: &CloudPlannerTarget,
    phase: &str,
    error: crate::inference::InferenceError,
) -> AgenticLoopError {
    let code = match error.code.as_str() {
        "credential_unavailable" => "credential_unavailable",
        "provider_network_error" => "provider_network_error",
        "provider_rate_limited" => "provider_rate_limited",
        "provider_response_error" | "provider_stream_interrupted_after_tokens" => {
            "provider_response_error"
        }
        _ => "cloud_planner_failed",
    };
    eprintln!(
        "CLOUD_PLANNER_FAILED phase={} provider={} model={} code={} boundary={}",
        phase, target.provider_id, target.model_id, error.code, error.boundary
    );
    AgenticLoopError {
        code,
        boundary: "AgentPlanning",
        message: format!(
            "Cloud planning could not complete with {}/{} during the {} pass. {} No action was executed.",
            target.provider_name, target.model_id, phase, error.message
        ),
        mlc_path: None,
    }
}

fn cloud_plan_repair_reason(objective: &str, draft: &GeneratedActionPlanDraft) -> Option<String> {
    if matches!(draft.source, IntentSource::Degraded) {
        return Some(
            draft
                .degraded_reason
                .clone()
                .unwrap_or_else(|| "ActionPlan JSON failed schema validation.".to_string()),
        );
    }
    plan_coverage::validate_objective_coverage(objective, draft)
        .err()
        .map(|deficit| deficit.message())
}
