use super::InferenceError;
use crate::{
    agent_manager::AgentManager,
    gemma::{
        classifier_protocol::{self, request as semantic_classifier_request},
        GemmaService,
    },
};
use std::{
    sync::{Arc, OnceLock},
    thread,
    time::{Duration, Instant},
};

mod cloud_route;
mod objective_policy;
pub(crate) mod private_apple_read;
mod semantic_assessment;
#[cfg(test)]
#[path = "dynamic_routing/sprint_304_tests.rs"]
mod sprint_304_tests;
use cloud_route::{configured_cloud_route, require_configured_cloud_route};
pub(crate) use cloud_route::{configured_cloud_route_snapshot, ConfiguredCloudRouteSnapshot};
use semantic_assessment::{
    SemanticAssessment, SemanticCapability, SemanticClassifierCode, SemanticConfidence,
    SemanticDemand, SemanticReason,
};

pub(crate) const SEMANTIC_CLASSIFIER_VERSION: &str = classifier_protocol::CLASSIFIER_VERSION;
pub(crate) const AUTO_ROUTE_POLICY_VERSION: &str = "auto_route_policy_v2";
const SEMANTIC_CLASSIFIER_QUEUE_TIMEOUT: Duration = Duration::from_secs(2);
const SEMANTIC_CLASSIFIER_PREPARATION_TIMEOUT: Duration = Duration::from_secs(90);
const SEMANTIC_CLASSIFIER_PROBE_TIMEOUT: Duration = Duration::from_secs(20);
const SEMANTIC_CLASSIFIER_INFERENCE_TIMEOUT: Duration = Duration::from_secs(12);
const SEMANTIC_CLASSIFIER_CLEANUP_GRACE: Duration = Duration::from_secs(1);
static SEMANTIC_CLASSIFIER_QUEUE: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone)]
pub(super) struct DynamicModelRouteDecision {
    pub local_provider_id: String,
    pub local_model_id: String,
    pub provider_id: String,
    pub model_id: String,
    pub matched_complexity_rules: Vec<String>,
    pub tier: &'static str,
    pub reason: String,
    pub classifier_source: String,
    pub capability: String,
    pub demand: String,
    pub confidence: String,
    pub classification_reason: String,
    pub classifier_latency_ms: u128,
    pub classifier_model_id: Option<String>,
    pub readiness_generation: u64,
    pub recovery_attempted: bool,
    pub policy_version: &'static str,
}

#[derive(Debug, Clone)]
pub(crate) struct PlannerDynamicRouteDecision {
    pub(crate) provider_id: String,
    pub(crate) model_id: String,
    pub(crate) reason: String,
    pub(crate) requires_cloud: bool,
}

pub(crate) async fn resolve_dynamic_planner_route(
    agent_manager: &AgentManager,
    gemma: &GemmaService,
    objective: &str,
    local_provider_id: &str,
    local_model_id: &str,
) -> Result<PlannerDynamicRouteDecision, InferenceError> {
    let decision = resolve_dynamic_model_route(
        agent_manager,
        gemma,
        objective,
        local_provider_id,
        local_model_id,
    )
    .await?;
    Ok(PlannerDynamicRouteDecision {
        provider_id: decision.provider_id,
        model_id: decision.model_id,
        reason: decision.reason,
        requires_cloud: decision.tier == "cloud_tier_2",
    })
}

#[derive(Debug, Clone)]
struct ClassifierOperationalError {
    code: &'static str,
    boundary: &'static str,
    message: String,
    recovery_attempted: bool,
}

impl ClassifierOperationalError {
    fn new(code: &'static str, boundary: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            boundary,
            message: message.into(),
            recovery_attempted: false,
        }
    }

    fn with_recovery_attempted(mut self) -> Self {
        self.recovery_attempted = true;
        self
    }

    fn into_inference_error(self) -> InferenceError {
        InferenceError::routing_attention(
            self.code,
            self.boundary,
            format!(
                "Auto-route couldn't choose a model. Nothing was sent to a provider. {}",
                self.message
            ),
        )
    }
}

pub(super) async fn resolve_dynamic_model_route(
    agent_manager: &AgentManager,
    gemma: &GemmaService,
    prompt: &str,
    local_provider_id: &str,
    local_model_id: &str,
) -> Result<DynamicModelRouteDecision, InferenceError> {
    let assessment = semantic_assessment_for_route(gemma, prompt).await?;
    resolve_dynamic_model_route_from_assessment(
        agent_manager,
        local_provider_id,
        local_model_id,
        assessment,
    )
}

pub(super) async fn resolve_dynamic_model_route_with_frozen_cloud(
    gemma: &GemmaService,
    prompt: &str,
    local_provider_id: &str,
    local_model_id: &str,
    cloud: Option<&ConfiguredCloudRouteSnapshot>,
) -> Result<DynamicModelRouteDecision, InferenceError> {
    let assessment = semantic_assessment_for_route(gemma, prompt).await?;
    resolve_dynamic_model_route_from_assessment_with_cloud(
        local_provider_id,
        local_model_id,
        assessment,
        require_configured_cloud_route(cloud),
    )
}

async fn semantic_assessment_for_route(
    gemma: &GemmaService,
    prompt: &str,
) -> Result<SemanticAssessment, InferenceError> {
    if objective_policy::deterministic_hydrated_public_grounding_applies(prompt) {
        return Ok(objective_policy::deterministic_local_assessment(
            objective_policy::HYDRATED_PUBLIC_GROUNDING_POLICY_VERSION,
            gemma.classifier_health().readiness_generation,
        ));
    }
    if objective_policy::deterministic_bounded_rewrite_applies(prompt)
        || objective_policy::deterministic_bounded_conversation_applies(prompt)
    {
        return Ok(objective_policy::deterministic_local_assessment(
            objective_policy::BOUNDED_LOCAL_POLICY_VERSION,
            gemma.classifier_health().readiness_generation,
        ));
    } else if objective_policy::deterministic_current_research_applies(prompt) {
        let health = gemma.classifier_health();
        return Ok(SemanticAssessment {
            demand: SemanticDemand::Advanced,
            capability: SemanticCapability::ResearchSynthesis,
            confidence: SemanticConfidence::Confident,
            reason: SemanticReason::SourceSynthesis,
            source: objective_policy::CURRENT_RESEARCH_POLICY_VERSION.to_string(),
            classifier_latency_ms: 0,
            classifier_model_id: None,
            readiness_generation: health.readiness_generation,
            recovery_attempted: false,
        });
    }

    Ok(objective_policy::apply_semantic_policy(
        classify_semantic_complexity(gemma, prompt).await?,
        prompt,
    ))
}

pub(super) fn resolve_explicit_dynamic_model_route(
    agent_manager: &AgentManager,
    gemma: &GemmaService,
    local_provider_id: &str,
    local_model_id: &str,
    choice: &str,
    cloud_confirmed: bool,
) -> Result<DynamicModelRouteDecision, InferenceError> {
    resolve_explicit_dynamic_model_route_with_cloud(
        gemma,
        local_provider_id,
        local_model_id,
        choice,
        cloud_confirmed,
        configured_cloud_route(agent_manager),
    )
}

pub(super) fn resolve_explicit_dynamic_model_route_with_frozen_cloud(
    gemma: &GemmaService,
    local_provider_id: &str,
    local_model_id: &str,
    choice: &str,
    cloud_confirmed: bool,
    cloud: Option<&ConfiguredCloudRouteSnapshot>,
) -> Result<DynamicModelRouteDecision, InferenceError> {
    resolve_explicit_dynamic_model_route_with_cloud(
        gemma,
        local_provider_id,
        local_model_id,
        choice,
        cloud_confirmed,
        require_configured_cloud_route(cloud),
    )
}

fn resolve_explicit_dynamic_model_route_with_cloud(
    gemma: &GemmaService,
    local_provider_id: &str,
    local_model_id: &str,
    choice: &str,
    cloud_confirmed: bool,
    cloud: Result<(String, String, String), InferenceError>,
) -> Result<DynamicModelRouteDecision, InferenceError> {
    let health = gemma.classifier_health();
    let assessment = SemanticAssessment {
        demand: if choice == "cloud" {
            SemanticDemand::Advanced
        } else {
            SemanticDemand::Routine
        },
        capability: SemanticCapability::Uncertain,
        confidence: SemanticConfidence::Confident,
        reason: SemanticReason::ExplicitUserChoice,
        source: format!("explicit_turn_choice_v1:{choice}"),
        classifier_latency_ms: 0,
        classifier_model_id: health.classifier_model_id,
        readiness_generation: health.readiness_generation,
        recovery_attempted: false,
    };
    let audit_signals = assessment.audit_signals();
    match choice {
        "local" => Ok(route_decision(
            local_provider_id,
            local_model_id,
            local_provider_id.to_string(),
            local_model_id.to_string(),
            audit_signals,
            "local_tier_1",
            "The user explicitly chose the saved local model for this turn after Auto-route needed attention."
                .to_string(),
            &assessment,
        )),
        "cloud" if cloud_confirmed => {
            let (provider_id, model_id, provider_name) = cloud?;
            Ok(route_decision(
                local_provider_id,
                local_model_id,
                provider_id,
                model_id,
                audit_signals,
                "cloud_tier_2",
                format!(
                    "The user explicitly confirmed that this turn may leave the device and chose the configured cloud target ({provider_name})."
                ),
                &assessment,
            ))
        }
        "cloud" => Err(InferenceError::routing_attention(
            "auto_route_cloud_confirmation_required",
            "auto_route_turn_choice",
            "Cloud use for this turn requires explicit confirmation that the message will leave the device. Nothing was sent.",
        )),
        _ => Err(InferenceError::routing_attention(
            "auto_route_turn_choice_invalid",
            "auto_route_turn_choice",
            "The Auto-route choice was not recognized. Nothing was sent to a provider.",
        )),
    }
}

async fn classify_semantic_complexity(
    gemma: &GemmaService,
    prompt: &str,
) -> Result<SemanticAssessment, InferenceError> {
    super::sprint_300_qualification::maybe_interrupt(gemma)?;
    let queue = SEMANTIC_CLASSIFIER_QUEUE.get_or_init(|| tokio::sync::Mutex::new(()));
    let _queue_guard = tokio::time::timeout(SEMANTIC_CLASSIFIER_QUEUE_TIMEOUT, queue.lock())
        .await
        .map_err(|_| {
            classifier_failure(
                gemma,
                "classifier_queue_timeout",
                "auto_route_classifier_queue",
                "The classifier was busy with another turn until this turn's routing deadline expired.",
                false,
            )
        })?;

    let mut recovery_attempted = false;
    if !gemma.classifier_health().is_ready() {
        recovery_attempted = true;
        recover_and_probe_classifier(gemma).await.map_err(|error| {
            classifier_failure(gemma, error.code, error.boundary, error.message, true)
        })?;
    }

    let started = Instant::now();
    match run_classifier_once(gemma, prompt).await {
        Ok(mut assessment) => {
            let health = gemma.classifier_health();
            assessment.classifier_latency_ms = started.elapsed().as_millis();
            assessment.classifier_model_id = health.classifier_model_id;
            assessment.readiness_generation = health.readiness_generation;
            assessment.recovery_attempted = recovery_attempted;
            Ok(assessment)
        }
        Err(error) => Err(classifier_failure(
            gemma,
            error.code,
            error.boundary,
            error.message,
            recovery_attempted,
        )),
    }
}

async fn run_classifier_once(
    gemma: &GemmaService,
    prompt: &str,
) -> Result<SemanticAssessment, ClassifierOperationalError> {
    let request = semantic_classifier_request(prompt);
    let cancellation = Arc::clone(&request.cancellation);
    let service = gemma.clone();
    let (sender, mut receiver) = tokio::sync::oneshot::channel();
    if thread::Builder::new()
        .name("oomu-semantic-classifier".to_string())
        .spawn(move || {
            let _ = sender.send(service.infer_classifier_sync(request));
        })
        .is_err()
    {
        return Err(ClassifierOperationalError::new(
            "classifier_worker_spawn_failed",
            "auto_route_classifier_worker",
            "The native classifier worker could not be started.",
        ));
    }

    match await_bounded_classifier_worker(
        &mut receiver,
        &cancellation,
        SEMANTIC_CLASSIFIER_INFERENCE_TIMEOUT,
        SEMANTIC_CLASSIFIER_CLEANUP_GRACE,
    )
    .await
    {
        Some(Ok(Ok(response))) => parse_semantic_classifier_output(&response.text)
            .map(SemanticAssessment::from_code)
            .map_err(|code| {
                ClassifierOperationalError::new(
                    code,
                    "auto_route_classifier_schema",
                    "The classifier returned output outside the routing grammar.",
                )
            }),
        Some(Ok(Err(error))) => Err(ClassifierOperationalError::new(
            classifier_error_code(error.code),
            "auto_route_classifier_inference",
            error.message,
        )),
        Some(Err(_)) => Err(ClassifierOperationalError::new(
            "classifier_worker_join_failed",
            "auto_route_classifier_worker",
            "The classifier worker ended without a result.",
        )),
        None => Err(ClassifierOperationalError::new(
            "classifier_timeout",
            "auto_route_classifier_worker",
            "The ready on-device classifier did not finish within its inference deadline.",
        )),
    }
}

async fn await_bounded_classifier_worker<T>(
    receiver: &mut tokio::sync::oneshot::Receiver<T>,
    cancellation: &Arc<std::sync::atomic::AtomicBool>,
    runtime_limit: Duration,
    cleanup_grace: Duration,
) -> Option<Result<T, tokio::sync::oneshot::error::RecvError>> {
    let cleanup_limit = cleanup_grace.min(runtime_limit);
    let inference_limit = runtime_limit.saturating_sub(cleanup_limit);
    match tokio::time::timeout(inference_limit, &mut *receiver).await {
        Ok(result) => Some(result),
        Err(_) => {
            cancellation.store(true, std::sync::atomic::Ordering::SeqCst);
            let _ = tokio::time::timeout(cleanup_limit, &mut *receiver).await;
            None
        }
    }
}

async fn recover_and_probe_classifier(
    gemma: &GemmaService,
) -> Result<u64, ClassifierOperationalError> {
    let recovery_epoch = gemma.mark_classifier_recovering();
    let service = gemma.clone();
    let preparation = tauri::async_runtime::spawn_blocking(move || {
        service.reload_configured_classifier_for_recovery(recovery_epoch)
    });
    match tokio::time::timeout(SEMANTIC_CLASSIFIER_PREPARATION_TIMEOUT, preparation).await {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(error))) => Err(ClassifierOperationalError::new(
            classifier_error_code(error.code),
            "auto_route_classifier_preparation",
            error.message,
        )
        .with_recovery_attempted())?,
        Ok(Err(error)) => Err(ClassifierOperationalError::new(
            "classifier_preparation_join_failed",
            "auto_route_classifier_preparation",
            error.to_string(),
        )
        .with_recovery_attempted())?,
        Err(_) => Err(ClassifierOperationalError::new(
            "classifier_preparation_timeout",
            "auto_route_classifier_preparation",
            "The on-device model did not finish preparing within its preparation deadline.",
        )
        .with_recovery_attempted())?,
    };

    let service = gemma.clone();
    let probe = tauri::async_runtime::spawn_blocking(move || {
        verify_classifier_readiness_for_recovery_sync(&service, recovery_epoch)
    });
    match tokio::time::timeout(SEMANTIC_CLASSIFIER_PROBE_TIMEOUT, probe).await {
        Ok(Ok(Ok(generation))) => Ok(generation),
        Ok(Ok(Err(error))) => Err(ClassifierOperationalError::new(
            classifier_error_code(error.code),
            "auto_route_classifier_probe",
            error.message,
        )
        .with_recovery_attempted()),
        Ok(Err(error)) => Err(ClassifierOperationalError::new(
            "classifier_probe_join_failed",
            "auto_route_classifier_probe",
            error.to_string(),
        )
        .with_recovery_attempted()),
        Err(_) => Err(ClassifierOperationalError::new(
            "classifier_probe_timeout",
            "auto_route_classifier_probe",
            "The prepared on-device model did not finish its readiness check in time.",
        )
        .with_recovery_attempted()),
    }
}

pub(crate) fn verify_classifier_readiness_for_recovery_sync(
    gemma: &GemmaService,
    recovery_epoch: u64,
) -> Result<u64, crate::gemma::GemmaError> {
    gemma.verify_classifier_readiness_for_recovery_sync(recovery_epoch)
}

fn classifier_failure(
    gemma: &GemmaService,
    code: &'static str,
    boundary: &'static str,
    message: impl Into<String>,
    recovery_attempted: bool,
) -> InferenceError {
    let message = message.into();
    gemma.mark_classifier_failure(code, boundary, &message);
    let mut error = ClassifierOperationalError::new(code, boundary, message);
    error.recovery_attempted = recovery_attempted;
    error.into_inference_error()
}

fn classifier_error_code(code: &str) -> &'static str {
    match code {
        "gemma_not_ready" => "classifier_not_ready",
        "local_model_incompatible" => "classifier_model_incompatible",
        "local_model_cancelled" | "native_inference_cancelled" => "classifier_cancelled",
        _ => "classifier_inference_failed",
    }
}

fn parse_semantic_classifier_output(text: &str) -> Result<SemanticClassifierCode, &'static str> {
    let code = classifier_protocol::validated_code(text)?;
    if code == "u" {
        return Ok(SemanticClassifierCode::Uncertain);
    }
    let bytes = code.as_bytes();
    if bytes.len() != 3 || bytes[2] != b'c' {
        return Err("classifier_schema_invalid");
    }
    let demand = match bytes[0] {
        b'r' => SemanticDemand::Routine,
        b'a' => SemanticDemand::Advanced,
        _ => return Err("classifier_schema_invalid"),
    };
    let capability = match bytes[1] {
        b'g' => SemanticCapability::General,
        b'm' => SemanticCapability::MathematicalReasoning,
        b'l' => SemanticCapability::LegalCompliance,
        b'a' => SemanticCapability::SystemArchitecture,
        b'r' => SemanticCapability::ResearchSynthesis,
        b'c' => SemanticCapability::CodeAnalysis,
        b'x' => SemanticCapability::MultiConstraintReasoning,
        b's' => SemanticCapability::SpecialistJudgment,
        _ => return Err("classifier_schema_invalid"),
    };
    Ok(SemanticClassifierCode::Classified { demand, capability })
}

fn resolve_dynamic_model_route_from_assessment(
    agent_manager: &AgentManager,
    local_provider_id: &str,
    local_model_id: &str,
    assessment: SemanticAssessment,
) -> Result<DynamicModelRouteDecision, InferenceError> {
    resolve_dynamic_model_route_from_assessment_with_cloud(
        local_provider_id,
        local_model_id,
        assessment,
        configured_cloud_route(agent_manager),
    )
}

fn resolve_dynamic_model_route_from_assessment_with_cloud(
    local_provider_id: &str,
    local_model_id: &str,
    assessment: SemanticAssessment,
    cloud: Result<(String, String, String), InferenceError>,
) -> Result<DynamicModelRouteDecision, InferenceError> {
    if !assessment.requires_cloud() {
        return resolve_local_model_route_from_assessment(
            local_provider_id,
            local_model_id,
            assessment,
        );
    }

    let audit_signals = assessment.audit_signals();
    let cloud_basis = assessment.cloud_basis();
    let (provider_id, model_id, provider_name) = cloud?;
    Ok(route_decision(
        local_provider_id,
        local_model_id,
        provider_id,
        model_id,
        audit_signals,
        "cloud_tier_2",
        format!(
            "{cloud_basis}; routing to database-configured Auto-route target ({}).",
            provider_name
        ),
        &assessment,
    ))
}

fn resolve_local_model_route_from_assessment(
    local_provider_id: &str,
    local_model_id: &str,
    assessment: SemanticAssessment,
) -> Result<DynamicModelRouteDecision, InferenceError> {
    if assessment.requires_cloud() {
        return Err(InferenceError::routing_attention(
            "auto_route_local_policy_invalid",
            "auto_route_local_policy",
            "OOMU could not verify this request as on-device work. Nothing was sent.",
        ));
    }
    let audit_signals = assessment.audit_signals();
    let reason = if private_apple_read::is_policy_source(&assessment.source) {
        "OOMU recognized a bounded private Apple-app read and kept it on the saved on-device model without contacting the difficulty classifier or a cloud provider."
            .to_string()
    } else if assessment.source == objective_policy::HYDRATED_PUBLIC_GROUNDING_POLICY_VERSION {
        "OOMU verified native public-search evidence before dispatch and kept the bounded completion on the saved local model without contacting the difficulty classifier or a cloud provider."
            .to_string()
    } else if assessment.source == objective_policy::BOUNDED_LOCAL_POLICY_VERSION {
        "OOMU's deterministic bounded-transformation policy matched this request; routing to the saved session local baseline without contacting a classifier or cloud provider."
            .to_string()
    } else {
        format!(
            "The local difficulty classifier returned a validated routine {} classification; routing to the saved session local baseline.",
            assessment.capability.as_str()
        )
    };
    Ok(route_decision(
        local_provider_id,
        local_model_id,
        local_provider_id.to_string(),
        local_model_id.to_string(),
        audit_signals,
        "local_tier_1",
        reason,
        &assessment,
    ))
}

fn route_decision(
    local_provider_id: &str,
    local_model_id: &str,
    provider_id: String,
    model_id: String,
    matched_complexity_rules: Vec<String>,
    tier: &'static str,
    reason: String,
    assessment: &SemanticAssessment,
) -> DynamicModelRouteDecision {
    DynamicModelRouteDecision {
        local_provider_id: local_provider_id.to_string(),
        local_model_id: local_model_id.to_string(),
        provider_id,
        model_id,
        matched_complexity_rules,
        tier,
        reason,
        classifier_source: assessment.source.clone(),
        capability: assessment.capability.as_str().to_string(),
        demand: assessment.demand.as_str().to_string(),
        confidence: assessment.confidence.as_str().to_string(),
        classification_reason: assessment.reason.as_str().to_string(),
        classifier_latency_ms: assessment.classifier_latency_ms,
        classifier_model_id: assessment.classifier_model_id.clone(),
        readiness_generation: assessment.readiness_generation,
        recovery_attempted: assessment.recovery_attempted,
        policy_version: AUTO_ROUTE_POLICY_VERSION,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_manager::ConfiguredProvider;
    use std::{path::PathBuf, sync::atomic::Ordering};

    const AUDITED_MATH_PROMPT: &str = "Develop a formal mathematical optimization model using semidefinite programming to schedule asynchronous data packets across a five-tier heterogeneous mesh network. Minimize end-to-end latency and energy use while enforcing bandwidth, fairness, queue stability, Byzantine-fault tolerance, and 99.999% delivery reliability. Explain the decision variables, feasibility constraints, objective tradeoffs, and validation under burst traffic.";
    const AUDITED_COMPLIANCE_PROMPT: &str = "Analyze a multi-national data processing agreement to identify latent compliance conflicts between GDPR Article 45 transfer mechanisms, California CPRA, Brazil LGPD, and Singapore PDPA. Reconcile incompatible retention, consent, data-residency, processor, and breach-notification duties; rank the legal exposure; and propose contract remediation without weakening any jurisdiction's protections.";

    fn manager(test_name: &str) -> (AgentManager, PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "oomu-semantic-routing-{test_name}-{}-{}.db",
            std::process::id(),
            crate::foundation::clock::unix_time_ns_u128()
        ));
        let manager = AgentManager::initialize_at(path.clone()).expect("manager initializes");
        (manager, path)
    }

    fn assessment(json: &str) -> SemanticAssessment {
        SemanticAssessment::from_code(
            parse_semantic_classifier_output(json).expect("semantic fixture parses"),
        )
    }

    fn configure_cloud_target(manager: &AgentManager) {
        manager
            .upsert_provider_config(ConfiguredProvider {
                id: "prov-semantic-target".to_string(),
                provider_id: "openrouter".to_string(),
                provider_name: "Configured Cloud".to_string(),
                auth_method: "api_key".to_string(),
                base_url: "https://openrouter.ai/api/v1".to_string(),
                api_key_label: "TEST_API_KEY".to_string(),
                api_key: Some("semantic-routing-test-key".to_string()),
                credential_configured: false,
                custom_model_ids: "cloud/model-primary, cloud/model-secondary".to_string(),
                auto_route_target: true,
                created_at_ms: 0,
                updated_at_ms: 0,
            })
            .expect("target saves");
    }

    #[tokio::test]
    async fn classifier_timeout_cancels_and_returns_after_bounded_cleanup() {
        let cancellation = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let released = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_cancellation = Arc::clone(&cancellation);
        let worker_released = Arc::clone(&released);
        let (sender, mut receiver) = tokio::sync::oneshot::channel();
        let worker = std::thread::spawn(move || {
            while !worker_cancellation.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(1));
            }
            worker_released.store(true, Ordering::Release);
            let _ = sender.send("cancelled");
        });
        let started = std::time::Instant::now();

        let result = await_bounded_classifier_worker(
            &mut receiver,
            &cancellation,
            Duration::from_millis(10),
            Duration::from_millis(10),
        )
        .await;

        assert!(result.is_none());
        assert!(cancellation.load(Ordering::SeqCst));
        worker.join().expect("blocking classifier worker exits");
        assert!(released.load(Ordering::Acquire));
        assert!(
            started.elapsed() < Duration::from_millis(250),
            "classifier cleanup exceeded its hard bound: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn classifier_queue_serializes_instead_of_returning_worker_busy() {
        let queue = SEMANTIC_CLASSIFIER_QUEUE.get_or_init(|| tokio::sync::Mutex::new(()));
        let first = queue.try_lock().expect("first classifier owns the queue");
        assert!(queue.try_lock().is_err());
        drop(first);
        assert!(queue.try_lock().is_ok());
    }

    #[test]
    fn classifier_recovery_phases_have_independent_deadlines() {
        assert_eq!(SEMANTIC_CLASSIFIER_QUEUE_TIMEOUT, Duration::from_secs(2));
        assert_eq!(
            SEMANTIC_CLASSIFIER_PREPARATION_TIMEOUT,
            Duration::from_secs(90)
        );
        assert_eq!(SEMANTIC_CLASSIFIER_PROBE_TIMEOUT, Duration::from_secs(20));
        assert_eq!(
            SEMANTIC_CLASSIFIER_INFERENCE_TIMEOUT,
            Duration::from_secs(12)
        );
        assert!(SEMANTIC_CLASSIFIER_PREPARATION_TIMEOUT > SEMANTIC_CLASSIFIER_INFERENCE_TIMEOUT);
        assert!(SEMANTIC_CLASSIFIER_PROBE_TIMEOUT > SEMANTIC_CLASSIFIER_INFERENCE_TIMEOUT);
    }

    #[test]
    fn audited_short_mathematical_and_compliance_prompts_route_cloud_semantically() {
        assert!(AUDITED_MATH_PROMPT.split_whitespace().count() < 180);
        assert!(AUDITED_COMPLIANCE_PROMPT.split_whitespace().count() < 180);
        assert!(semantic_classifier_request(AUDITED_MATH_PROMPT)
            .prompt
            .contains(AUDITED_MATH_PROMPT));
        assert!(semantic_classifier_request(AUDITED_COMPLIANCE_PROMPT)
            .prompt
            .contains(AUDITED_COMPLIANCE_PROMPT));

        let (manager, path) = manager("audited-prompts");
        configure_cloud_target(&manager);
        for (prompt, fixture, expected_capability) in [
            (AUDITED_MATH_PROMPT, r#""amc""#, "mathematical_reasoning"),
            (AUDITED_COMPLIANCE_PROMPT, r#""alc""#, "legal_compliance"),
        ] {
            let route = resolve_dynamic_model_route_from_assessment(
                &manager,
                "local_model",
                "gemma-4-12B-it-qat-q4_0-gguf",
                assessment(fixture),
            )
            .expect("cloud route resolves");
            assert!(!prompt.is_empty());
            assert_eq!(route.tier, "cloud_tier_2");
            assert_eq!(route.provider_id, "prov-semantic-target");
            assert_eq!(route.model_id, "cloud/model-primary");
            assert_eq!(route.capability, expected_capability);
            assert!(!route
                .matched_complexity_rules
                .iter()
                .any(|signal| signal.contains("word_count") || signal.starts_with("phrase:")));
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn ship_gate_semantic_floor_promotes_advanced_work_without_using_length_or_tool_words() {
        let routine = assessment(r#""rgc""#);
        for (prompt, capability, reason) in [
            (
                "Analyze a multinational data-processing agreement for conflicts among GDPR international-transfer duties, California CPRA, Brazil LGPD, and Singapore PDPA. Reconcile retention, consent, residency, processor, and breach-notification requirements; rank exposure; and propose contract remediation without weakening any jurisdiction's protections.",
                SemanticCapability::LegalCompliance,
                SemanticReason::HighStakesJudgment,
            ),
            (
                "Research current primary or official sources on scheduled/background agent capabilities in OpenClaw and Claude Cowork. Write a sourced comparison and explain what this implies for OOMU.",
                SemanticCapability::ResearchSynthesis,
                SemanticReason::SourceSynthesis,
            ),
            (
                "Construct a recovery plan that minimizes completion time while respecting dependencies, one-owner capacity, business hours, a 20% contingency reserve, and the requirement that security validation precede release validation. Write the critical path.",
                SemanticCapability::MultiConstraintReasoning,
                SemanticReason::CrossConstraintAnalysis,
            ),
        ] {
            let promoted = objective_policy::apply_semantic_floor(routine.clone(), prompt);
            assert_eq!(promoted.demand, SemanticDemand::Advanced);
            assert_eq!(promoted.capability, capability);
            assert_eq!(promoted.reason, reason);
            assert!(promoted
                .source
                .contains(objective_policy::OBJECTIVE_SEMANTIC_FLOOR_VERSION));
        }
    }

    #[test]
    fn semantic_floor_leaves_bounded_local_ship_gate_turns_routine() {
        for prompt in [
            "Read q3_strategic_vendor_proposals.txt and summarize only the stated facts in exactly three bullets. Do not use the internet.",
            "Remember these temporary test values for this chat only: cedar 14, indigo 22, quartz 31. Reply stored.",
            "The colour label is blue. Replace every occurrence of colour with color. Make no other change and do not explain.",
        ] {
            let unchanged =
                objective_policy::apply_semantic_floor(assessment(r#""rgc""#), prompt);
            assert_eq!(unchanged.demand, SemanticDemand::Routine);
            assert_eq!(unchanged.capability, SemanticCapability::General);
            assert!(!unchanged
                .source
                .contains(objective_policy::OBJECTIVE_SEMANTIC_FLOOR_VERSION));
        }
    }

    #[test]
    fn bounded_local_policy_corrects_length_only_or_tool_word_escalation() {
        for prompt in [
            "Read q3_strategic_vendor_proposals.txt and summarize only the stated facts in exactly three bullets. Do not recommend a vendor and do not use the internet.",
            "Remember these temporary test values for this chat only: cedar 14, indigo 22, quartz 31. Reply stored.",
            "Return those three names alphabetically with their values, one per line.",
            "The colour label is blue. Replace every occurrence of colour with color. Make no other change and do not explain.",
        ] {
            let corrected =
                objective_policy::apply_semantic_policy(assessment(r#""axc""#), prompt);
            assert_eq!(corrected.demand, SemanticDemand::Routine);
            assert_eq!(corrected.capability, SemanticCapability::General);
            assert!(corrected
                .source
                .contains(objective_policy::BOUNDED_LOCAL_POLICY_VERSION));
        }
    }

    #[test]
    fn bounded_conversation_policy_covers_the_profile_and_residency_contract() {
        for prompt in [
            "In one sentence, explain what the Command key does on a Mac.",
            "In one sentence, explain what the Command key does on a Mac. Remember that the project word for this chat is Cedar.",
            "Give me three common Mac shortcuts.",
            "Which shortcut opens Spotlight?",
            "How do I switch between open apps?",
            "How do I close the current window?",
            "How do I quit an app?",
            "How do I take a screenshot of part of the screen?",
            "How do I open Finder?",
            "What is the Mac equivalent of Control-Alt-Delete?",
            "How do I preview a file quickly in Finder?",
            "Summarize the shortcuts we discussed in five bullets.",
            "Which two should I learn first, and what was the project word?",
            "Which key did I ask about before OOMU restarted?",
        ] {
            assert!(
                objective_policy::deterministic_bounded_conversation_applies(prompt),
                "expected bounded local conversation: {prompt}"
            );
        }
    }

    #[test]
    fn bounded_conversation_policy_does_not_absorb_consequential_or_external_work() {
        for prompt in [
            "In one sentence, explain the latest official security guidance for this Mac.",
            "How do I diagnose chest pain and choose a medical treatment?",
            "What is the legal exposure if I delete these files?",
            "Give me three primary sources and research the current macOS release.",
            "How do I run the command that erases this Mac?",
            "What's on my calendar today?",
        ] {
            assert!(
                !objective_policy::deterministic_bounded_conversation_applies(prompt),
                "must remain on the classifier or native-tool policy path: {prompt}"
            );
        }
    }

    #[tokio::test]
    async fn canonical_thirty_repetition_rewrite_bypasses_classifier_and_stays_local() {
        let source = "The colour label is blue.\n".repeat(30);
        let prompt = format!(
            "{source}Replace every occurrence of `colour` with `color`. Make no other change and do not explain."
        );
        assert!(objective_policy::deterministic_bounded_rewrite_applies(
            &prompt
        ));

        // A disabled service makes this a production-path proof that the
        // deterministic rewrite does not enter classifier readiness, recovery,
        // queueing, or inference before selecting the local baseline.
        let gemma = GemmaService::new_disabled("classifier unavailable by test contract");
        let cloud = ConfiguredCloudRouteSnapshot {
            provider_id: "prov-must-not-be-used".to_string(),
            model_id: Some("gemini-3.5-flash".to_string()),
            provider_name: "Configured Cloud".to_string(),
            credential_configured: true,
        };
        let route = resolve_dynamic_model_route_with_frozen_cloud(
            &gemma,
            &prompt,
            "local_model",
            "gemma-4-E4B-it-qat-q4_0-gguf",
            Some(&cloud),
        )
        .await
        .expect("bounded rewrite selects local without invoking the disabled classifier");

        assert_eq!(route.provider_id, "local_model");
        assert_eq!(route.model_id, "gemma-4-E4B-it-qat-q4_0-gguf");
        assert_eq!(route.tier, "local_tier_1");
        assert_eq!(
            route.classifier_source,
            objective_policy::BOUNDED_LOCAL_POLICY_VERSION
        );
        assert_eq!(route.classifier_latency_ms, 0);
        assert!(!route.recovery_attempted);
        assert!(route.reason.contains("without contacting a classifier"));
    }

    #[tokio::test]
    async fn source_bound_current_research_bypasses_classifier_and_routes_cloud() {
        let prompt = "I'm trying to decide whether it's worth updating Rust right now. Could you look online to find the latest stable Rust release, then check the official release notes for that exact version and tell me whether it includes any newly stabilized language features? Give me a short recommendation with the version, release date, one example if there is one, and links to the official pages you used.";
        assert!(objective_policy::deterministic_current_research_applies(
            prompt
        ));

        let gemma = GemmaService::new_disabled("classifier unavailable by test contract");
        let cloud = ConfiguredCloudRouteSnapshot {
            provider_id: "prov-google-gemini".to_string(),
            model_id: Some("gemini-3.6-flash".to_string()),
            provider_name: "Google Gemini".to_string(),
            credential_configured: true,
        };
        let route = resolve_dynamic_model_route_with_frozen_cloud(
            &gemma,
            prompt,
            "local_model",
            "gemma-4-E4B-it-qat-q4_0-gguf",
            Some(&cloud),
        )
        .await
        .expect("current multi-stage research selects the configured cloud route");

        assert_eq!(route.provider_id, "prov-google-gemini");
        assert_eq!(route.model_id, "gemini-3.6-flash");
        assert_eq!(route.tier, "cloud_tier_2");
        assert_eq!(route.capability, "research_synthesis");
        assert_eq!(
            route.classifier_source,
            objective_policy::CURRENT_RESEARCH_POLICY_VERSION
        );
        assert_eq!(route.classifier_latency_ms, 0);
        assert!(!route.recovery_attempted);
        assert!(route
            .reason
            .contains("deterministic current-research policy"));
    }

    #[test]
    fn deterministic_rewrite_bypass_requires_the_native_bounded_contract() {
        let excessive = format!(
            "{}Replace every occurrence of colour with color. Make no other change and do not explain.",
            "The colour label is blue. ".repeat(513)
        );
        let trailing_work = "The colour label is blue. Replace every occurrence of colour with color. Make no other change and do not explain. Then research current paint standards.";

        assert!(!objective_policy::deterministic_bounded_rewrite_applies(
            &excessive
        ));
        assert!(!objective_policy::deterministic_bounded_rewrite_applies(
            trailing_work
        ));
    }

    #[test]
    fn easy_long_prompt_stays_on_the_configured_12b_model() {
        let (manager, path) = manager("easy-long-local");
        let prompt = "Please change each occurrence of colour to color. ".repeat(200);
        assert!(prompt.split_whitespace().count() > 180);
        let route = resolve_dynamic_model_route_from_assessment(
            &manager,
            "local_model",
            "gemma-4-12B-it-qat-q4_0-gguf",
            assessment(r#""rgc""#),
        )
        .expect("local route resolves");

        assert_eq!(route.provider_id, "local_model");
        assert_eq!(route.model_id, "gemma-4-12B-it-qat-q4_0-gguf");
        assert_eq!(route.tier, "local_tier_1");
        assert_eq!(route.demand, "routine");
        assert_eq!(route.confidence, "confident");
        assert!(route.reason.contains("validated routine general"));
        assert!(semantic_classifier_request(&prompt)
            .prompt
            .contains("[bounded middle omitted]"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn configured_cloud_target_is_required_without_system_fallback() {
        let (manager, path) = manager("configured-target");
        let missing = resolve_dynamic_model_route_from_assessment(
            &manager,
            "local_model",
            "gemma-4-12B-it-qat-q4_0-gguf",
            assessment(r#""aac""#),
        )
        .expect_err("missing target stops without a fallback");
        assert_eq!(missing.code, "auto_route_cloud_target_missing");

        configure_cloud_target(&manager);
        let configured = resolve_dynamic_model_route_from_assessment(
            &manager,
            "local_model",
            "gemma-4-12B-it-qat-q4_0-gguf",
            assessment(r#""arc""#),
        )
        .expect("configured route resolves");
        assert_eq!(configured.provider_id, "prov-semantic-target");
        assert_eq!(configured.model_id, "cloud/model-primary");
        assert!(configured.reason.contains("Configured Cloud"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn classifier_failure_stops_and_semantic_uncertainty_can_use_cloud() {
        let (manager, path) = manager("fail-closed");
        let operational = ClassifierOperationalError::new(
            "classifier_timeout",
            "auto_route_classifier_worker",
            "timed out",
        )
        .into_inference_error();
        assert_eq!(operational.code, "classifier_timeout");
        assert!(operational.message.contains("Nothing was sent"));

        configure_cloud_target(&manager);
        let route = resolve_dynamic_model_route_from_assessment(
            &manager,
            "local_model",
            "gemma-4-12B-it-qat-q4_0-gguf",
            assessment(r#""u""#),
        )
        .expect("grammar-valid uncertainty is a semantic cloud decision");
        assert_eq!(route.tier, "cloud_tier_2");
        assert_eq!(route.capability, "uncertain");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn explicit_attention_choices_bind_only_the_selected_turn() {
        let (manager, path) = manager("explicit-turn-choice");
        configure_cloud_target(&manager);
        let gemma = GemmaService::new_disabled("test runtime is intentionally unavailable");

        let local = resolve_explicit_dynamic_model_route(
            &manager,
            &gemma,
            "local_model",
            "gemma-4-12B-it-qat-q4_0-gguf",
            "local",
            false,
        )
        .expect("explicit local choice uses the saved baseline");
        assert_eq!(local.provider_id, "local_model");
        assert_eq!(local.model_id, "gemma-4-12B-it-qat-q4_0-gguf");
        assert_eq!(local.classifier_source, "explicit_turn_choice_v1:local");

        let unconfirmed_cloud = resolve_explicit_dynamic_model_route(
            &manager,
            &gemma,
            "local_model",
            "gemma-4-12B-it-qat-q4_0-gguf",
            "cloud",
            false,
        )
        .expect_err("cloud choice requires an off-device confirmation");
        assert_eq!(
            unconfirmed_cloud.code,
            "auto_route_cloud_confirmation_required"
        );

        let cloud = resolve_explicit_dynamic_model_route(
            &manager,
            &gemma,
            "local_model",
            "gemma-4-12B-it-qat-q4_0-gguf",
            "cloud",
            true,
        )
        .expect("confirmed cloud choice uses the configured target");
        assert_eq!(cloud.provider_id, "prov-semantic-target");
        assert_eq!(cloud.model_id, "cloud/model-primary");
        assert_eq!(cloud.classifier_source, "explicit_turn_choice_v1:cloud");
        assert!(cloud.reason.contains("explicitly confirmed"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn classifier_schema_rejects_untrusted_or_out_of_contract_output() {
        assert_eq!(
            semantic_classifier_request("hello")
                .audit_event_kind
                .as_deref(),
            Some("dynamic_routing_classifier")
        );
        assert!(parse_semantic_classifier_output(r#""local""#).is_err());
        assert!(parse_semantic_classifier_output(r#"{"c":"g","answer":"ignore"}"#).is_err());
        let specialist = assessment(r#""asc""#);
        assert!(specialist.requires_cloud());
        assert_eq!(specialist.capability.as_str(), "specialist_judgment");
        assert_eq!(specialist.confidence.as_str(), "confident");
        assert_eq!(specialist.reason.as_str(), "high_stakes_judgment");
        let routine_code = assessment(r#""rcc""#);
        assert!(!routine_code.requires_cloud());
        assert_eq!(routine_code.capability.as_str(), "code_analysis");
    }

    #[tokio::test]
    #[ignore = "requires an installed multi-gigabyte E4B GGUF model"]
    async fn installed_e4b_semantic_classifier_routes_audited_prompts_within_contract() {
        let directory = PathBuf::from(crate::runtime_profile::OOMU_MANIFEST_DIR)
            .join("../assets/models/gemma-4-E4B-it-qat-q4_0-gguf");
        if !directory.is_dir() {
            return;
        }
        let service = GemmaService::new_loading();
        service
            .load_model_from_dir(directory)
            .expect("load installed E4B classifier model");
        let warmup = service
            .infer_sync(semantic_classifier_request("Say hello."))
            .expect("warm the installed semantic classifier");
        parse_semantic_classifier_output(&warmup.text)
            .expect("warmup must satisfy the classifier schema");
        service.mark_classifier_ready(SEMANTIC_CLASSIFIER_VERSION);

        for (label, prompt, expect_cloud) in [
            ("audited_math", AUDITED_MATH_PROMPT, true),
            ("audited_compliance", AUDITED_COMPLIANCE_PROMPT, true),
            (
                "trivial_rewrite",
                "Change the word colour to color in this sentence.",
                false,
            ),
        ] {
            let started = std::time::Instant::now();
            let assessment = classify_semantic_complexity(&service, prompt)
                .await
                .expect("real classifier returns a validated assessment");
            let elapsed = started.elapsed();
            eprintln!(
                "SEMANTIC_CLASSIFIER_EVAL label={label} route={} capability={} elapsed_ms={}",
                if assessment.requires_cloud() {
                    "cloud"
                } else {
                    "local"
                },
                assessment.capability.as_str(),
                elapsed.as_millis()
            );
            assert_eq!(
                assessment.source, SEMANTIC_CLASSIFIER_VERSION,
                "the real classifier must not reach its fail-closed timeout/error path"
            );
            assert_eq!(
                assessment.requires_cloud(),
                expect_cloud,
                "prompt: {prompt}"
            );
            assert!(
                elapsed <= SEMANTIC_CLASSIFIER_INFERENCE_TIMEOUT,
                "semantic classification exceeded its runtime contract"
            );
        }
    }

    #[tokio::test]
    #[ignore = "requires the installed multi-gigabyte OOMU E4B model"]
    async fn installed_e4b_real_auto_route_corpus() {
        const OBJECTIVE: &str = "prepare a board-ready supplier decision pack. Read /Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/mocked_data/supplier_proposals.json and q3_strategic_vendor_proposals.txt from my testing folder. Reconcile every quoted amount and margin, identify all exceptions, and independently research current primary or official web sources for fuel or freight conditions that could materially affect the recommendation. Cite every web claim with its URL and access time. Create a new ship_test_01 folder in the testing folder and deliver four real files: supplier_decision.xlsx, supplier_decision.pptx, supplier_decision.pdf, and sources.md. The workbook must contain source data, formulas, exception flags, and a recommendation sheet. The presentation and PDF must be executive-ready and mutually consistent. Then create a tentative 30-minute event in my OOMU Test calendar on the next weekday between 1:00 PM and 4:00 PM titled Supplier Decision Review, avoiding conflicts, and create a Mail draft to recipient@example.com summarizing the recommendation and listing the four output files. Do not send the email. Ask for any required approvals and continue from the exact stopped step after I approve. Do not claim completion until you have verified that all four files, the calendar event, and the unsent Mail draft actually exist.";
        let directory = PathBuf::from(crate::runtime_profile::OOMU_MANIFEST_DIR)
            .join("../assets/models/gemma-4-E4B-it-qat-q4_0-gguf");
        assert!(directory.is_dir(), "installed OOMU E4B model is present");
        let service = GemmaService::new_loading();
        service
            .load_model_from_dir(directory)
            .expect("load installed E4B classifier model");
        service
            .verify_classifier_readiness_sync()
            .expect("E4B classifier passes the readiness probe");

        let corpus = [
            ("incident_greeting", "Hello OOMU", false),
            (
                "incident_conversation",
                "We are working on bug fixes for you today",
                false,
            ),
            (
                "routine_fact",
                "What is the capital of France?",
                false,
            ),
            (
                "routine_rewrite",
                "Change the word colour to color in this sentence.",
                false,
            ),
            ("routine_math", "What is 17 plus 25?", false),
            (
                "routine_code",
                "In JavaScript, rename the local variable totalCount to count in this one-line statement: const totalCount = items.length;",
                false,
            ),
            ("advanced_math", AUDITED_MATH_PROMPT, true),
            ("advanced_compliance", AUDITED_COMPLIANCE_PROMPT, true),
            ("advanced_ship_readiness", OBJECTIVE, true),
        ];
        let mut timings_ms = Vec::with_capacity(corpus.len());
        for (label, prompt, expect_cloud) in corpus {
            let started = std::time::Instant::now();
            let assessment = classify_semantic_complexity(&service, prompt)
                .await
                .unwrap_or_else(|error| {
                    panic!(
                        "{label}: real E4B classification failed: {} at {}",
                        error.code, error.boundary
                    )
                });
            let elapsed_ms = started.elapsed().as_millis();
            timings_ms.push(elapsed_ms);
            eprintln!(
                "AUTO_ROUTE_REAL_CORPUS label={label} route={} capability={} demand={} elapsed_ms={elapsed_ms}",
                if assessment.requires_cloud() {
                    "cloud"
                } else {
                    "local"
                },
                assessment.capability.as_str(),
                assessment.demand.as_str(),
            );
            assert_eq!(assessment.source, SEMANTIC_CLASSIFIER_VERSION);
            assert_eq!(
                assessment.requires_cloud(),
                expect_cloud,
                "must-route mismatch for {label}: {prompt}"
            );
        }
        timings_ms.sort_unstable();
        let percentile = |percent: usize| {
            let index = ((timings_ms.len() - 1) * percent).div_ceil(100);
            timings_ms[index]
        };
        eprintln!(
            "AUTO_ROUTE_REAL_CORPUS_TIMING model=gemma-4-E4B-it-qat-q4_0-gguf classifier={} samples={} p50_ms={} p95_ms={} p99_ms={}",
            SEMANTIC_CLASSIFIER_VERSION,
            timings_ms.len(),
            percentile(50),
            percentile(95),
            percentile(99),
        );
    }
}
