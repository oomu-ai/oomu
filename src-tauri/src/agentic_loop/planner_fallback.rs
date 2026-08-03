use super::*;

pub(super) const CLOUD_PLANNER_LOCAL_FALLBACK_MODEL_ID: &str = "gemma-4-E4B-it-qat-q4_0-gguf";
pub(super) const CLOUD_PLANNER_LOCAL_FALLBACK_MODEL_NAME: &str = "Gemma 4 E4B";

fn generate_local_plan_draft(
    objective: String,
    compiled_prompt: String,
    retry_prompt: Option<String>,
    gemma: GemmaService,
) -> GeneratedActionPlanDraft {
    let draft = match gemma.infer_sync(local_planner_infer_request(compiled_prompt)) {
        Ok(response) => generated_plan_from_text(objective.clone(), response.text),
        Err(error) => {
            let mut draft = generated_plan_from_text(objective.clone(), String::new());
            draft.degraded_reason = Some(format!("Local planner degraded: {}", error.message));
            draft
        }
    };
    retry_local_planner_draft_once(draft, retry_prompt, |prompt| {
        gemma
            .infer_sync(local_planner_infer_request(prompt))
            .ok()
            .map(|response| generated_plan_from_text(objective, response.text))
    })
}

fn generate_explicit_local_plan_draft(
    objective: String,
    compiled_prompt: String,
    model_id: String,
    gemma: GemmaService,
) -> GeneratedActionPlanDraft {
    match gemma.infer_model_sync(&model_id, local_planner_infer_request(compiled_prompt)) {
        Ok(response) => generated_plan_from_text(objective, response.text),
        Err(error) => {
            eprintln!(
                "CLOUD_PLANNER_LOCAL_FALLBACK_FAILED boundary=AgentPlanning model={} code={}",
                model_id, error.code
            );
            let mut draft = generated_plan_from_text(objective, String::new());
            draft.degraded_reason = Some(format!(
                "Local fallback planner was unavailable: {}",
                error.code
            ));
            draft
        }
    }
}

pub(super) fn should_retry_local_planner(
    draft: &GeneratedActionPlanDraft,
    optional_context_was_bounded: bool,
) -> bool {
    optional_context_was_bounded && matches!(draft.source, IntentSource::Degraded)
}

pub(super) fn retry_local_planner_draft_once<F>(
    draft: GeneratedActionPlanDraft,
    retry_prompt: Option<String>,
    retry: F,
) -> GeneratedActionPlanDraft
where
    F: FnOnce(String) -> Option<GeneratedActionPlanDraft>,
{
    if !should_retry_local_planner(&draft, retry_prompt.is_some()) {
        return draft;
    }
    eprintln!("LOCAL_PLANNER_MINIMAL_RETRY boundary=AgentPlanning reason=bounded_optional_context");
    retry_prompt.and_then(retry).unwrap_or(draft)
}

pub(super) fn cloud_planner_local_fallback_target(
    error: &AgenticLoopError,
) -> Option<PlannerExecutionTarget> {
    (error.code == "planner_output_unusable").then(|| PlannerExecutionTarget::Local {
        model_id: Some(CLOUD_PLANNER_LOCAL_FALLBACK_MODEL_ID.to_string()),
        reason: "The selected cloud planner exhausted its verified schema repair, so OOMU completed planning locally with Gemma 4 E4B."
            .to_string(),
    })
}

pub(super) fn finalize_cloud_planner_local_fallback(
    objective: &str,
    draft: GeneratedActionPlanDraft,
    original_error: AgenticLoopError,
    fallback_target: PlannerExecutionTarget,
) -> Result<(GeneratedActionPlanDraft, PlannerExecutionTarget), AgenticLoopError> {
    let rejection_code = if matches!(draft.source, IntentSource::Degraded) {
        Some("parse_or_schema_validation_failed")
    } else if draft.steps.is_empty() {
        Some("empty_action_plan")
    } else {
        plan_coverage::validate_objective_coverage(objective, &draft)
            .err()
            .map(|deficit| deficit.code())
    };
    if let Some(rejection_code) = rejection_code {
        eprintln!(
            "CLOUD_PLANNER_LOCAL_FALLBACK_EXHAUSTED boundary=AgentPlanning model={} reason={}",
            CLOUD_PLANNER_LOCAL_FALLBACK_MODEL_ID, rejection_code
        );
        return Err(original_error);
    }
    Ok((draft, fallback_target))
}

pub(super) async fn generate_plan_draft(
    objective: String,
    planning_sections: PlannerPromptSections,
    gemma: GemmaService,
    target: PlannerExecutionTarget,
) -> Result<(GeneratedActionPlanDraft, PlannerExecutionTarget), AgenticLoopError> {
    generate_plan_draft_with_validation_objective(objective, planning_sections, gemma, target).await
}

pub(super) async fn generate_composition_plan_draft(
    validation_objective: String,
    planning_sections: PlannerPromptSections,
    gemma: GemmaService,
    target: PlannerExecutionTarget,
) -> Result<(GeneratedActionPlanDraft, PlannerExecutionTarget), AgenticLoopError> {
    generate_plan_draft_with_validation_objective(
        validation_objective,
        planning_sections,
        gemma,
        target,
    )
    .await
}

async fn generate_plan_draft_with_validation_objective(
    validation_objective: String,
    planning_sections: PlannerPromptSections,
    gemma: GemmaService,
    target: PlannerExecutionTarget,
) -> Result<(GeneratedActionPlanDraft, PlannerExecutionTarget), AgenticLoopError> {
    let planning_objective = planning_sections.objective.clone();
    match target {
        local_target @ PlannerExecutionTarget::Local { .. } => {
            let compiled = compile_planner_prompt(&planning_sections)?;
            if compiled.optional_context_bounded {
                eprintln!(
                    "PLANNER_OPTIONAL_CONTEXT_BOUNDED boundary=AgentPlanning envelope_tokens={PLANNER_INPUT_TOKEN_LIMIT}"
                );
            }
            let retry_prompt = if compiled.optional_context_bounded {
                Some(minimal_local_planner_retry_prompt(
                    &planning_sections.objective,
                )?)
            } else {
                None
            };
            let draft = tauri::async_runtime::spawn_blocking(move || {
                generate_local_plan_draft(
                    validation_objective,
                    compiled.prompt,
                    retry_prompt,
                    gemma,
                )
            })
            .await
            .map_err(|error| AgenticLoopError {
                code: "intent_worker_join_failed",
                boundary: "GemmaService",
                message: error.to_string(),
                mlc_path: None,
            })?;
            Ok((draft, local_target))
        }
        PlannerExecutionTarget::Cloud(cloud_target) => {
            let prompt = compile_cloud_planner_prompt(&planning_objective)?;
            let requested_target = PlannerExecutionTarget::Cloud(cloud_target.clone());
            match generate_cloud_plan_draft(validation_objective.clone(), prompt, cloud_target)
                .await
            {
                Ok(draft) => Ok((draft, requested_target)),
                Err(original_error) => {
                    let Some(fallback_target) =
                        cloud_planner_local_fallback_target(&original_error)
                    else {
                        return Err(original_error);
                    };
                    let fallback_prompt = match minimal_local_planner_retry_prompt(
                        &planning_objective,
                    ) {
                        Ok(prompt) => prompt,
                        Err(error) => {
                            eprintln!(
                                "CLOUD_PLANNER_LOCAL_FALLBACK_EXHAUSTED boundary=AgentPlanning model={} reason={}",
                                CLOUD_PLANNER_LOCAL_FALLBACK_MODEL_ID, error.code
                            );
                            return Err(original_error);
                        }
                    };
                    eprintln!(
                        "CLOUD_PLANNER_LOCAL_FALLBACK_STARTED boundary=AgentPlanning model={}",
                        CLOUD_PLANNER_LOCAL_FALLBACK_MODEL_ID
                    );
                    let objective_for_fallback = validation_objective.clone();
                    let fallback_model_id = CLOUD_PLANNER_LOCAL_FALLBACK_MODEL_ID.to_string();
                    let draft = match tauri::async_runtime::spawn_blocking(move || {
                        generate_explicit_local_plan_draft(
                            objective_for_fallback,
                            fallback_prompt,
                            fallback_model_id,
                            gemma,
                        )
                    })
                    .await
                    {
                        Ok(draft) => draft,
                        Err(error) => {
                            eprintln!(
                                "CLOUD_PLANNER_LOCAL_FALLBACK_EXHAUSTED boundary=AgentPlanning model={} reason=worker_join_failed detail={}",
                                CLOUD_PLANNER_LOCAL_FALLBACK_MODEL_ID,
                                compact_for_prompt(&error.to_string(), 160)
                            );
                            return Err(original_error);
                        }
                    };
                    finalize_cloud_planner_local_fallback(
                        &validation_objective,
                        draft,
                        original_error,
                        fallback_target,
                    )
                }
            }
        }
    }
}
