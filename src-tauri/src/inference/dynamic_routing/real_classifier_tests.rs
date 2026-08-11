use super::*;
use crate::gemma::{
    resolve_verified_startup_model_assignment, GemmaService, StartupModelPreference,
    StartupModelSelectionSource,
};
use std::path::PathBuf;

const AUDITED_MATH_PROMPT: &str = "Develop a formal mathematical optimization model using semidefinite programming to schedule asynchronous data packets across a five-tier heterogeneous mesh network. Minimize end-to-end latency and energy use while enforcing bandwidth, fairness, queue stability, Byzantine-fault tolerance, and 99.999% delivery reliability.";
const AUDITED_COMPLIANCE_PROMPT: &str = "Analyze a multi-national data processing agreement to identify latent compliance conflicts between GDPR Article 45 transfer mechanisms, California CPRA, Brazil LGPD, and Singapore PDPA. Reconcile incompatible retention, consent, data-residency, processor, and breach-notification duties; rank the legal exposure; and propose contract remediation.";

fn installed_classifier() -> Option<GemmaService> {
    let directory = std::env::var_os("OOMU_CLASSIFIER_TEST_MODEL_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(crate::runtime_profile::OOMU_MANIFEST_DIR)
                .join("../assets/models")
                .join(crate::gemma::GEMMA_E2B_CANONICAL_ID)
        });
    if !directory.is_dir() {
        return None;
    }
    let model_root = directory
        .parent()
        .expect("installed classifier model has a parent directory");
    let requested_model_id = directory
        .file_name()
        .and_then(|name| name.to_str())
        .expect("installed classifier model directory has a UTF-8 name");
    let assignment = resolve_verified_startup_model_assignment(
        model_root,
        &StartupModelPreference {
            requested_model_id: requested_model_id.to_string(),
            selection_source: StartupModelSelectionSource::ExplicitUserSelection,
        },
    )
    .expect("resolve installed classifier model assignment");
    let service = GemmaService::new_loading();
    service
        .reconfigure_startup_model_assignment(assignment)
        .expect("load installed classifier lane");
    service
        .verify_classifier_readiness_sync()
        .expect("classifier passes the readiness probe");
    Some(service)
}

#[tokio::test]
#[ignore = "requires the installed multi-gigabyte OOMU classifier model"]
async fn installed_classifier_real_auto_route_corpus() {
    let Some(service) = installed_classifier() else {
        return;
    };
    let corpus = [
        ("greeting", "Hello OOMU", false),
        ("conversation", "We are working on bug fixes for you today", false),
        ("fact", "What is the capital of France?", false),
        ("rewrite", "Change the word colour to color in this sentence.", false),
        ("math", "What is 17 plus 25?", false),
        ("advanced_math", AUDITED_MATH_PROMPT, true),
        ("advanced_compliance", AUDITED_COMPLIANCE_PROMPT, true),
        (
            "supplier_tradeoffs",
            "Perform a comprehensive strategic evaluation across two private supplier datasets. Compare technical compliance, unit pricing, and delivery risks, reconcile conflicting requirements, and provide a multi-scenario vendor trade-off matrix.",
            true,
        ),
        (
            "supplier_tradeoffs_with_named_private_inputs",
            "Perform a comprehensive strategic evaluation of the supplier proposals in alpha_private_input.json and cross-reference them with the requirements in beta_private_requirements.txt located in \"/Users/example/Documents/OOMU/Projects/private_inputs\". Compare technical compliance, unit pricing, and delivery risks, and provide a multi-scenario vendor trade-off matrix.",
            true,
        ),
        (
            "explicit_cloud",
            "Use the configured cloud model for this complex multi-file evaluation and reconcile the conflicting requirements before recommending a vendor.",
            true,
        ),
        (
            "localized_semantics",
            "Concilie dos propuestas privadas, compare el cumplimiento técnico, el precio unitario y el riesgo de entrega, y presente una matriz de escenarios con una recomendación justificada.",
            true,
        ),
    ];
    for (label, prompt, expect_cloud) in corpus {
        let assessment = classify_semantic_complexity(&service, prompt)
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "{label}: real classification failed: {} at {}",
                    error.code, error.boundary
                )
            });
        eprintln!(
            "AUTO_ROUTE_REAL_CORPUS label={label} route={} capability={} demand={}",
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
}
