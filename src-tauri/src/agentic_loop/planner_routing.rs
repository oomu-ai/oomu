use super::*;

pub(super) fn active_cloud_planner_target(
    agent_manager: Option<&AgentManager>,
    _objective: &str,
    _unified_preference: bool,
) -> Result<Option<PlannerExecutionTarget>, AgenticLoopError> {
    let Some(manager) = agent_manager else {
        return Ok(None);
    };
    let Some(target) = manager.get_active_auto_route_target().map_err(|_| {
        planner_configuration_error(
            "planner_provider_configuration_failed",
            "OOMU could not read the Auto-route provider configuration. Check it in Settings and try again.",
        )
    })? else {
        return Ok(None);
    };
    let config = manager
        .select_provider_config(&target.id)
        .map_err(|_| planner_configuration_error(
            "planner_provider_configuration_failed",
            "OOMU could not open the Auto-route provider credentials. Check the provider in Settings and try again.",
        ))?
        .ok_or_else(|| planner_configuration_error(
            "planner_provider_configuration_failed",
            "The Auto-route provider is no longer configured. Choose another provider and try again.",
        ))?;

    let reason = "Using {provider_name}/{model_id} for this request.".to_string();

    cloud_planner_target_from_config(&config, None, reason).map(Some)
}

pub(super) fn planner_configuration_error(code: &'static str, message: &str) -> AgenticLoopError {
    AgenticLoopError {
        code,
        boundary: "AgentPlanning",
        message: message.to_string(),
        mlc_path: None,
    }
}

pub(super) fn selected_cloud_planner_target(
    agent_manager: Option<&AgentManager>,
    selected_provider_id: Option<&str>,
    selected_model_id: Option<&str>,
) -> Result<Option<PlannerExecutionTarget>, AgenticLoopError> {
    let Some(provider_config_id) = selected_provider_id.and_then(clean_planner_text) else {
        return Ok(None);
    };
    if provider_config_id.eq_ignore_ascii_case("dynamic") {
        return Ok(None);
    }
    if provider_id_is_local(&provider_config_id) {
        return Ok(Some(PlannerExecutionTarget::Local {
            model_id: selected_model_id.and_then(clean_planner_text),
            reason: "Kept your on-device model.".to_string(),
        }));
    }
    if matches!(
        provider_config_id.to_ascii_lowercase().as_str(),
        "gemini" | "google" | "google_gemini" | "gemini_pro" | "gemini_flash"
    ) {
        let model_id = selected_model_id
            .and_then(clean_planner_text)
            .unwrap_or_else(|| DYNAMIC_CLOUD_FALLBACK_MODEL_ID.to_string());
        let reason = format!("Using Google Gemini/{model_id} for this request.");
        return Ok(Some(PlannerExecutionTarget::Cloud(CloudPlannerTarget {
            provider_config_id: None,
            provider_id: "gemini".to_string(),
            provider_name: "Google Gemini".to_string(),
            model_id,
            base_url: None,
            api_key_label: None,
            api_key: None,
            reason,
        })));
    }
    let Some(manager) = agent_manager else {
        return Err(planner_configuration_error(
            "planner_provider_configuration_failed",
            "The selected cloud provider is unavailable. Choose a configured provider and try again.",
        ));
    };

    let provider_metadata = manager
        .select_provider_configs()
        .map_err(|_| planner_configuration_error(
            "planner_provider_configuration_failed",
            "OOMU could not read the selected provider configuration. Check the provider in Settings and try again.",
        ))?
        .into_iter()
        .find(|provider| provider.id == provider_config_id)
        .ok_or_else(|| planner_configuration_error(
            "planner_provider_configuration_failed",
            "The selected cloud provider is no longer configured. Choose another provider and try again.",
        ))?;
    if provider_id_is_local(&provider_metadata.provider_id) {
        return Ok(Some(PlannerExecutionTarget::Local {
            model_id: selected_model_id.and_then(clean_planner_text),
            reason: "Unified planner resolved the selected provider configuration to its on-device model."
                .to_string(),
        }));
    }
    let config = manager
        .select_provider_config(&provider_config_id)
        .map_err(|_| planner_configuration_error(
            "planner_provider_configuration_failed",
            "OOMU could not open the selected provider credentials. Check the provider in Settings and try again.",
        ))?
        .ok_or_else(|| planner_configuration_error(
            "planner_provider_configuration_failed",
            "The selected cloud provider is no longer configured. Choose another provider and try again.",
        ))?;
    let reason = "Using {provider_name}/{model_id} for this request.".to_string();
    cloud_planner_target_from_config(&config, selected_model_id, reason).map(Some)
}

pub(super) fn resolve_planning_execution_target(
    agent_manager: Option<&AgentManager>,
    objective: &str,
    preference: ModelRoutePreference,
    selected_provider_id: Option<&str>,
    selected_model_id: Option<&str>,
) -> Result<PlannerExecutionTarget, AgenticLoopError> {
    if let Some(target) =
        selected_cloud_planner_target(agent_manager, selected_provider_id, selected_model_id)?
    {
        return bind_specialist_provider_config(agent_manager, objective, target);
    }

    if matches!(
        preference,
        ModelRoutePreference::GeminiPro | ModelRoutePreference::ChatGpt
    ) {
        if let Some(target) = active_cloud_planner_target(agent_manager, objective, true)? {
            return bind_specialist_provider_config(agent_manager, objective, target);
        }
    }

    let demands_cloud_planning = planner_demands_cloud_planning(objective);
    if demands_cloud_planning {
        if let Some(target) = active_cloud_planner_target(agent_manager, objective, false)? {
            return bind_specialist_provider_config(agent_manager, objective, target);
        }
    }

    let reason = if demands_cloud_planning {
        "Local Gemma selected because the objective matched systems-programming planner signals, but no configured Auto-Route cloud target is available."
            .to_string()
    } else if matches!(
        preference,
        ModelRoutePreference::GeminiPro | ModelRoutePreference::ChatGpt
    ) {
        format!(
            "Local Gemma selected because cloud planner escalation is reserved for systems-programming complexity signals; requested route was {preference:?}."
        )
    } else {
        "Local Gemma selected for deterministic local planning.".to_string()
    };

    bind_specialist_provider_config(
        agent_manager,
        objective,
        PlannerExecutionTarget::Local {
            model_id: None,
            reason,
        },
    )
}

fn bind_specialist_provider_config(
    agent_manager: Option<&AgentManager>,
    objective: &str,
    target: PlannerExecutionTarget,
) -> Result<PlannerExecutionTarget, AgenticLoopError> {
    bind_required_specialist_provider_config(
        agent_manager,
        plan_coverage::deterministic_draft_requires_dynamic_route(objective),
        target,
    )
}

pub(super) fn bind_specialist_draft_provider_config(
    agent_manager: Option<&AgentManager>,
    draft: &GeneratedActionPlanDraft,
    target: PlannerExecutionTarget,
) -> Result<PlannerExecutionTarget, AgenticLoopError> {
    let required = draft.steps.iter().any(|step| {
        matches!(
            &step.tool,
            GeneratedToolDraft::RegisteredTaskTool { operation, .. }
                if matches!(
                    operation.trim(),
                    crate::tools::evidence_artifacts::COMPARISON_OPERATION
                        | crate::tools::evidence_artifacts::RECOVERY_OPERATION
                )
        )
    });
    bind_required_specialist_provider_config(agent_manager, required, target)
}

fn bind_required_specialist_provider_config(
    agent_manager: Option<&AgentManager>,
    required: bool,
    target: PlannerExecutionTarget,
) -> Result<PlannerExecutionTarget, AgenticLoopError> {
    if !required {
        return Ok(target);
    }
    let PlannerExecutionTarget::Cloud(alias_target) = target else {
        return Err(specialist_route_repair_error());
    };
    if alias_target.provider_config_id.is_some() {
        return Ok(PlannerExecutionTarget::Cloud(alias_target));
    }
    let Some(manager) = agent_manager else {
        return Err(specialist_route_repair_error());
    };
    let providers = manager.select_provider_configs().map_err(|_| {
        planner_configuration_error(
            "planner_provider_configuration_failed",
            "OOMU could not read the selected provider configuration. Check the provider in Settings and try again.",
        )
    })?;
    let Some(alias_family) = provider_alias_family(&alias_target.provider_id) else {
        return Err(specialist_route_repair_error());
    };
    let compatible = providers
        .iter()
        .filter(|provider| provider_alias_family(&provider.provider_id) == Some(alias_family))
        .collect::<Vec<_>>();
    let model_matches = compatible
        .iter()
        .copied()
        .filter(|provider| provider_lists_model(provider, &alias_target.model_id))
        .collect::<Vec<_>>();
    let preferred = if model_matches.is_empty() {
        &compatible
    } else {
        &model_matches
    };
    let provider = if let [provider] = preferred.as_slice() {
        *provider
    } else {
        let auto_route = preferred
            .iter()
            .copied()
            .filter(|provider| provider.auto_route_target)
            .collect::<Vec<_>>();
        let [provider] = auto_route.as_slice() else {
            return Err(specialist_route_repair_error());
        };
        *provider
    };
    let manual_label = alias_target.provider_name;
    let manual_reason = alias_target.reason;
    let exact = cloud_planner_target_from_config(
        provider,
        Some(&alias_target.model_id),
        manual_reason.clone(),
    )?;
    let PlannerExecutionTarget::Cloud(mut exact) = exact else {
        return Err(specialist_route_repair_error());
    };
    exact.provider_name = manual_label;
    exact.reason = manual_reason;
    Ok(PlannerExecutionTarget::Cloud(exact))
}

fn specialist_route_repair_error() -> AgenticLoopError {
    planner_configuration_error(
        "planner_provider_configuration_failed",
        "The selected cloud provider is unavailable. Choose a configured provider and try again.",
    )
}

fn provider_alias_family(provider_id: &str) -> Option<&'static str> {
    match provider_id
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .as_str()
    {
        "gemini" | "google" | "google_gemini" | "gemini_pro" | "gemini_flash" => {
            Some("google_gemini")
        }
        _ => None,
    }
}

fn provider_lists_model(provider: &ConfiguredProvider, model_id: &str) -> bool {
    let requested = model_id.trim().trim_start_matches("models/");
    provider
        .custom_model_ids
        .split(|character| character == ',' || character == '\n')
        .map(str::trim)
        .map(|model| model.trim_start_matches("models/"))
        .any(|model| model.eq_ignore_ascii_case(requested))
}

fn cloud_planner_target_from_config(
    config: &ConfiguredProvider,
    selected_model_id: Option<&str>,
    reason_template: String,
) -> Result<PlannerExecutionTarget, AgenticLoopError> {
    if provider_id_is_local(&config.provider_id) {
        return Err(planner_configuration_error(
            "planner_provider_configuration_failed",
            "Auto-route requires a cloud provider. Choose a cloud provider in Settings and try again.",
        ));
    }

    let provider_id = clean_planner_text(&config.provider_id).ok_or_else(|| {
        planner_configuration_error(
            "planner_provider_configuration_failed",
            "The selected provider has no valid provider ID. Repair it in Settings and try again.",
        )
    })?;
    crate::agent_manager::canonical_provider_secret_origin(&provider_id, &config.base_url)
        .map_err(|_| planner_configuration_error(
            "planner_provider_configuration_failed",
            "The selected provider endpoint is not approved for stored credentials. Repair it in Settings and try again.",
        ))?;
    let api_key = config.api_key.as_deref().and_then(clean_planner_text);
    let api_key_label = clean_planner_text(&config.api_key_label);
    let normalized_provider_id = provider_id.trim().to_ascii_lowercase().replace('-', "_");
    let model_id = selected_model_id
        .and_then(clean_planner_text)
        .or_else(|| first_planner_model_id(&config.custom_model_ids))
        .ok_or_else(|| planner_configuration_error(
            "planner_cloud_model_unavailable",
            "The selected provider has no planning model configured. Choose a model in Settings and try again.",
        ))?;
    let provider_name = clean_planner_text(&config.provider_name).unwrap_or(provider_id.clone());
    let reason = reason_template
        .replace("{provider_name}", &provider_name)
        .replace("{model_id}", &model_id);

    Ok(PlannerExecutionTarget::Cloud(CloudPlannerTarget {
        provider_config_id: Some(config.id.clone()),
        provider_id,
        provider_name,
        model_id,
        base_url: if normalized_provider_id == "custom" {
            clean_planner_text(&config.base_url)
        } else {
            None
        },
        api_key_label,
        api_key,
        reason,
    }))
}

pub(super) fn planner_demands_cloud_planning(objective: &str) -> bool {
    !planner_cloud_signals(objective).is_empty()
}

pub(super) fn plain_plan_route_reason(
    selected_model: &ModelMetadata,
    project_note_count: usize,
    includes_recent_chat: bool,
) -> String {
    let model = if selected_model.provider.eq_ignore_ascii_case("local") {
        selected_model.name.clone()
    } else {
        format!("{}/{}", selected_model.provider, selected_model.name)
    };
    match (project_note_count, includes_recent_chat) {
        (0, false) => format!("Using {model} for this request."),
        (0, true) => format!("Using {model} for this request with your recent chat."),
        (count, false) => {
            format!("Using {model} for this request with {count} notes from this project.")
        }
        (count, true) => format!(
            "Using {model} for this request with {count} notes from this project and your recent chat."
        ),
    }
}

fn planner_cloud_signals(objective: &str) -> Vec<&'static str> {
    let normalized = objective.to_lowercase();
    let mut signals = [
        "unsafe",
        "unsafe rust",
        "allocator",
        "memory allocator",
        "linker",
        "dynamic linker",
        "kernel",
        "wasm",
        "assembly",
        "concurrency",
        "mutex",
        "lock-free",
        "lock free",
        "borrow checker",
        "memory model",
        "systems programming",
        "ffi",
    ]
    .into_iter()
    .filter(|signal| normalized.contains(signal))
    .collect::<Vec<_>>();
    signals.sort_unstable();
    signals.dedup();
    signals
}

pub(super) fn first_planner_model_id(custom_model_ids: &str) -> Option<String> {
    custom_model_ids
        .split(|character| character == ',' || character == '\n')
        .map(str::trim)
        .find(|model_id| !model_id.is_empty())
        .map(str::to_string)
}

pub(super) fn clean_planner_text(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

pub(super) fn provider_id_is_local(provider_id: &str) -> bool {
    matches!(
        provider_id
            .trim()
            .replace('-', "_")
            .to_ascii_lowercase()
            .as_str(),
        "local" | "local_model" | "local_gemma"
    )
}

pub(super) async fn resolve_objective_planner_route(
    request: &AgentObjectiveRequest,
    agent: &AgentConfig,
    agent_manager: &AgentManager,
    gemma: &GemmaService,
    persistence: &PersistenceEngine,
    objective: &str,
) -> Result<Option<crate::inference::dynamic_routing::PlannerDynamicRouteDecision>, AgenticLoopError>
{
    let requested = request.dynamic_routing_enabled
        || request
            .selected_provider_id
            .as_deref()
            .is_some_and(|provider_id| provider_id.eq_ignore_ascii_case("dynamic"))
        || request
            .selected_model_id
            .as_deref()
            .is_some_and(|model_id| model_id.eq_ignore_ascii_case("dynamic"));
    if !requested {
        return Ok(None);
    }
    let (local_provider_id, local_model_id) = resolve_local_planner_baseline(
        agent_manager,
        agent,
        persistence,
        request.session_id.as_deref(),
    )?;
    crate::inference::dynamic_routing::resolve_dynamic_planner_route(
        agent_manager,
        gemma,
        objective,
        &local_provider_id,
        &local_model_id,
    )
    .await
    .map(Some)
    .map_err(|error| AgenticLoopError {
        code: "dynamic_planner_route_failed",
        boundary: "AgentPlanning",
        message: error.message,
        mlc_path: None,
    })
}

fn resolve_local_planner_baseline(
    agent_manager: &AgentManager,
    agent: &AgentConfig,
    persistence: &PersistenceEngine,
    session_id: Option<&str>,
) -> Result<(String, String), AgenticLoopError> {
    let (configured_provider_id, configured_model_id) =
        planner_baseline_fields(persistence, session_id, agent)?;
    let provider_id = if planner_provider_is_local(&configured_provider_id) {
        configured_provider_id
    } else {
        let providers = agent_manager.select_provider_configs().map_err(|_| {
            dynamic_route_error(
                "OOMU could not verify the Auto-route local provider. Check the active agent in Settings and try again.",
            )
        })?;
        let provider = providers
            .into_iter()
            .find(|provider| provider.id == configured_provider_id)
            .ok_or_else(|| {
                dynamic_route_error(
                    "Auto-route requires this chat to use a configured local baseline model.",
                )
            })?;
        if !planner_provider_is_local(&provider.provider_id) {
            return Err(dynamic_route_error(
                "Auto-route requires this chat to use a configured local baseline model.",
            ));
        }
        provider.provider_id
    };
    let model_root = crate::settings::resolved_local_model_directory_headless();
    let model = crate::gemma::resolve_strict_local_model(&model_root, &configured_model_id)
        .map_err(|error| dynamic_route_error(&error.message))?;
    Ok((provider_id, model.id))
}

pub(crate) fn planner_baseline_fields(
    persistence: &PersistenceEngine,
    session_id: Option<&str>,
    agent: &AgentConfig,
) -> Result<(String, String), AgenticLoopError> {
    let Some(session_id) = session_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok((agent.provider_id.clone(), agent.model_id.clone()));
    };
    let policy = persistence
        .select_chat_session_route_policy(session_id)
        .map_err(|_| {
            dynamic_route_error(
                "OOMU could not read this chat’s saved Auto-route model. Reopen the chat and try again.",
            )
        })?
        .ok_or_else(|| {
            dynamic_route_error(
                "This chat no longer has a saved Auto-route model. Choose a local model and try again.",
            )
        })?;
    if policy.agent_id != agent.id {
        return Err(dynamic_route_error(
            "This chat’s saved Auto-route model belongs to a different agent. Reopen the intended chat and try again.",
        ));
    }
    let provider_id = policy
        .local_provider_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("dynamic"))
        .ok_or_else(|| {
            dynamic_route_error(
                "This Auto-route chat has no saved local provider. Choose a local model and try again.",
            )
        })?;
    let model_id = policy
        .local_model_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("dynamic"))
        .ok_or_else(|| {
            dynamic_route_error(
                "This Auto-route chat has no saved local model. Choose a local model and try again.",
            )
        })?;
    Ok((provider_id.to_string(), model_id.to_string()))
}

fn planner_provider_is_local(provider_id: &str) -> bool {
    matches!(
        provider_id
            .trim()
            .replace('-', "_")
            .to_ascii_lowercase()
            .as_str(),
        "local" | "local_model" | "local_gemma"
    )
}

fn dynamic_route_error(message: &str) -> AgenticLoopError {
    AgenticLoopError {
        code: "dynamic_planner_route_failed",
        boundary: "AgentPlanning",
        message: message.to_string(),
        mlc_path: None,
    }
}
