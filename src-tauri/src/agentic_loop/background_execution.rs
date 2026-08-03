use super::*;
use futures_util::FutureExt;
use std::panic::AssertUnwindSafe;

pub(super) fn completion_registration(
    request: &AgentPlanExecutionRequest,
    execution_id: &str,
    locale: String,
) -> crate::gateway::auto_turn::AutoTurnRegistration {
    let (provider_id, model_id) = exact_completion_route(
        &request.plan.model_route,
        &request.turn_context.provider_id,
        &request.turn_context.model_id,
    );
    crate::gateway::auto_turn::AutoTurnRegistration {
        callback: crate::gateway::auto_turn::AutoTurnCallback {
            session_id: request.turn_context.session_id.clone(),
            task_id: execution_id.to_string(),
            injector_prompt_template: "The verified background task {task_id} completed with this native execution result:\n{data}".to_string(),
        },
        agent_id: request.turn_context.agent_id.clone(),
        provider_id,
        model_id,
        parent_turn_id: request.turn_context.turn_id.clone(),
        root_turn_id: request.turn_context.root_turn_id.clone(),
        locale,
        automated_web_grounding_enabled: request.turn_context.automated_web_grounding_enabled,
        // A completion belongs to the already-signed execution. Reclassifying its
        // receipt can switch models after work has completed or demand a cloud model
        // that the user never selected. Keep the completion on the exact signed lane.
        dynamic_routing_override: Some(false),
    }
}

fn exact_completion_route(
    route: &ModelRouteDecision,
    fallback_provider_id: &str,
    fallback_model_id: &str,
) -> (String, String) {
    if !fallback_provider_id.trim().is_empty() && !fallback_model_id.trim().is_empty() {
        return (
            fallback_provider_id.to_string(),
            fallback_model_id.to_string(),
        );
    }
    let provider_id = if route.selected_model.locality == "remote" {
        route
            .provider_config_id
            .as_ref()
            .or(route.provider_id.as_ref())
    } else {
        route.provider_id.as_ref()
    }
    .filter(|value| !value.trim().is_empty())
    .cloned()
    .unwrap_or_else(|| fallback_provider_id.to_string());
    let model_id = (!route.selected_model.name.trim().is_empty())
        .then(|| route.selected_model.name.clone())
        .unwrap_or_else(|| fallback_model_id.to_string());
    (provider_id, model_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_route_is_exactly_the_signed_cloud_provider_and_model() {
        let route = ModelRouteDecision {
            selected_model: ModelMetadata {
                name: "gemini-3.5-flash".to_string(),
                version: "API bridge".to_string(),
                provider: "Google Gemini".to_string(),
                locality: "remote".to_string(),
            },
            provider_config_id: Some("provider-config-7".to_string()),
            provider_id: Some("gemini".to_string()),
            recommended_model: None,
            requires_principal_authorization: false,
            reason: "fixture".to_string(),
            context_excerpt_count: 0,
            context_sources: Vec::new(),
        };
        assert_eq!(
            exact_completion_route(&route, "provider-config-7", "gemini-3.5-flash"),
            (
                "provider-config-7".to_string(),
                "gemini-3.5-flash".to_string()
            )
        );
    }

    #[test]
    fn completion_route_stays_on_the_exact_signed_local_model() {
        let route = ModelRouteDecision {
            selected_model: ModelMetadata {
                name: "gemma-4-E2B-it-qat-q4_0-gguf".to_string(),
                version: "local".to_string(),
                provider: "Local".to_string(),
                locality: "local".to_string(),
            },
            provider_config_id: None,
            provider_id: Some("local_model".to_string()),
            recommended_model: None,
            requires_principal_authorization: false,
            reason: "fixture".to_string(),
            context_excerpt_count: 0,
            context_sources: Vec::new(),
        };
        assert_eq!(
            exact_completion_route(&route, "", ""),
            (
                "local_model".to_string(),
                "gemma-4-E2B-it-qat-q4_0-gguf".to_string(),
            )
        );
    }

    #[test]
    fn completion_route_cannot_replace_its_immutable_parent_model() {
        let route = ModelRouteDecision {
            selected_model: ModelMetadata {
                name: "Gemma 4 E4B".to_string(),
                version: "local".to_string(),
                provider: "Local".to_string(),
                locality: "local".to_string(),
            },
            provider_config_id: None,
            provider_id: Some("local_model".to_string()),
            recommended_model: None,
            requires_principal_authorization: false,
            reason: "fixture".to_string(),
            context_excerpt_count: 0,
            context_sources: Vec::new(),
        };
        assert_eq!(
            exact_completion_route(&route, "local_model", "gemma-4-E2B-it-qat-q4_0-gguf",),
            (
                "local_model".to_string(),
                "gemma-4-E2B-it-qat-q4_0-gguf".to_string(),
            )
        );
    }
}

pub(super) fn register_completion(
    app: &tauri::AppHandle,
    registration: &crate::gateway::auto_turn::AutoTurnRegistration,
) -> Result<(), AgenticLoopError> {
    app.state::<crate::gateway::auto_turn::AutoTurnRegistry>()
        .register(registration.clone())
        .map_err(|message| AgenticLoopError {
            code: "auto_turn_registration_failed",
            boundary: "AutoTurnRegistry",
            message,
            mlc_path: None,
        })
}

pub(super) fn cancel_completion(app: &tauri::AppHandle, execution_id: &str) {
    app.state::<crate::gateway::auto_turn::AutoTurnRegistry>()
        .cancel(execution_id);
}

#[allow(clippy::too_many_arguments)]
pub(super) fn spawn(
    request: AgentPlanExecutionRequest,
    agent: AgentConfig,
    persistence: PersistenceEngine,
    memory_ledger: MemoryLedger,
    identity: SovereignIdentity,
    gemma: GemmaService,
    execution_id: String,
    leases: ActuationLeaseManager,
    app: tauri::AppHandle,
    origin_guard: AgentExecutionOriginGuard,
    registration: crate::gateway::auto_turn::AutoTurnRegistration,
) {
    crate::gateway::auto_turn::emit_registered(&app, &registration.callback);
    let panic_plan = request.plan.clone();
    let panic_session_id = request.turn_context.session_id.clone();
    let panic_agent_id = request.turn_context.agent_id.clone();
    let panic_origin_guard = origin_guard.clone();
    let callback = registration.callback.clone();
    let worker_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = AssertUnwindSafe(run_agent_plan_execution(
            request,
            agent,
            persistence,
            Some(memory_ledger),
            identity,
            gemma,
            Some(execution_id.clone()),
            Some(leases),
            Some(worker_app),
            origin_guard,
        ))
        .catch_unwind()
        .await;
        match result {
            Ok(Ok(success)) => deliver_success(&app, &execution_id, &callback, success).await,
            Ok(Err(_)) => deliver_native_failure(&app, &execution_id, &callback),
            Err(payload) => {
                handle_agent_execution_panic(
                    &app,
                    &execution_id,
                    &panic_plan,
                    &panic_session_id,
                    &panic_agent_id,
                    &panic_origin_guard,
                    payload,
                );
                deliver_native_failure(&app, &execution_id, &callback);
            }
        }
    });
}

async fn deliver_success(
    app: &tauri::AppHandle,
    execution_id: &str,
    callback: &crate::gateway::auto_turn::AutoTurnCallback,
    success: AgenticLoopResponse,
) {
    let completion_data = serde_json::json!({
        "status": "completed",
        "verified": success.verified,
        "outputs": success.outputs,
    })
    .to_string();
    let registry = app.state::<crate::gateway::auto_turn::AutoTurnRegistry>();
    let dispatcher = crate::gateway::auto_turn::NativeAutoTurnDispatcher::new(app.clone());
    if let Err(error) = registry
        .complete(execution_id, completion_data, &dispatcher)
        .await
    {
        crate::gateway::auto_turn::emit_failed(app, callback);
        eprintln!(
            "OOMU_AUTO_TURN_DISPATCH_FAILED execution_id_hash={} error={}",
            sha256_hex(execution_id.as_bytes()),
            error
        );
    }
}

fn deliver_native_failure(
    app: &tauri::AppHandle,
    execution_id: &str,
    callback: &crate::gateway::auto_turn::AutoTurnCallback,
) {
    cancel_completion(app, execution_id);
    crate::gateway::auto_turn::emit_failed(app, callback);
}
