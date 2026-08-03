use super::{
    SemanticAssessment, SemanticCapability, SemanticConfidence, SemanticDemand, SemanticReason,
};

pub(super) const OBJECTIVE_SEMANTIC_FLOOR_VERSION: &str = "objective_semantic_floor_v1";
pub(super) const BOUNDED_LOCAL_POLICY_VERSION: &str = "bounded_local_policy_v2";
pub(super) const CURRENT_RESEARCH_POLICY_VERSION: &str = "deterministic_current_research_v1";
pub(super) const HYDRATED_PUBLIC_GROUNDING_POLICY_VERSION: &str = "hydrated_public_grounding_v1";

pub(super) fn deterministic_local_assessment(
    source: &'static str,
    readiness_generation: u64,
) -> SemanticAssessment {
    SemanticAssessment {
        demand: SemanticDemand::Routine,
        capability: SemanticCapability::General,
        confidence: SemanticConfidence::Confident,
        reason: SemanticReason::BoundedTransformation,
        source: source.to_string(),
        classifier_latency_ms: 0,
        classifier_model_id: None,
        readiness_generation,
        recovery_attempted: false,
    }
}

pub(super) fn deterministic_bounded_rewrite_applies(prompt: &str) -> bool {
    crate::gemma::deterministic_transform::has_bounded_exact_rewrite_contract(prompt)
}

/// A native search receipt proves that public retrieval already completed
/// before provider dispatch. Finishing a bounded answer from that hydrated
/// evidence stays on the saved local baseline and must not depend on classifier
/// readiness or silently promote the public context to a cloud provider.
pub(super) fn deterministic_hydrated_public_grounding_applies(prompt: &str) -> bool {
    let normalized = prompt.to_ascii_lowercase();
    normalized.contains("verified-native-public-grounding: true")
        && normalized.contains("page title")
        && contains_any(&normalized, &[" link", " url"])
        && !contains_any(
            &normalized,
            &[
                "analyze",
                "compare",
                "cross-check",
                "synthesize",
                "then check",
                "recommendation",
            ],
        )
}

/// Short, ordinary product-help questions and bounded references to the
/// current conversation do not need a probabilistic difficulty decision. The
/// narrow exclusions keep consequential judgment, research, and actuation on
/// the normal classifier and approval paths.
pub(super) fn deterministic_bounded_conversation_applies(prompt: &str) -> bool {
    let normalized = prompt.trim().to_ascii_lowercase();
    if normalized.is_empty()
        || normalized.chars().count() > 320
        || contains_any(
            &normalized,
            &[
                "analyze",
                "diagnose",
                "research",
                "search online",
                "search the web",
                "look online",
                "latest",
                "official source",
                "primary source",
                "legal",
                "medical",
                "financial",
                "security",
                "delete this",
                "delete these",
                "delete the file",
                "delete all",
                "erase",
                "send an email",
                "run the command",
                "execute the command",
                "write the file",
                "create the file",
                "calendar",
            ],
        )
    {
        return false;
    }

    let bounded_request = contains_any(
        &normalized,
        &[
            "in one sentence",
            "give me three",
            "in five bullets",
            "which shortcut",
            "which two should i learn",
        ],
    ) || normalized.starts_with("how do i ")
        || normalized.starts_with("what is the ")
        || normalized.starts_with("summarize the ")
        || normalized.starts_with("which key did i ask");
    let ordinary_mac_help = contains_any(
        &normalized,
        &[
            "command key",
            "mac shortcut",
            "shortcut",
            "spotlight",
            "open apps",
            "current window",
            "quit an app",
            "screenshot",
            "finder",
            "control-alt-delete",
            "preview a file",
        ],
    );
    let current_chat_recall = contains_any(
        &normalized,
        &[
            "did i ask about",
            "what did i ask",
            "we discussed",
            "project word",
            "before oomu restarted",
        ],
    );

    bounded_request && (ordinary_mac_help || current_chat_recall)
}

/// Current, source-bound, multi-stage research is unambiguously cloud-tier
/// work. Recognizing that narrow contract before local classifier inference
/// prevents a cold or busy classifier from blocking a request whose route is
/// already mechanically determined by the user's words.
pub(super) fn deterministic_current_research_applies(prompt: &str) -> bool {
    let normalized = prompt.to_ascii_lowercase();
    let external_retrieval = contains_any(
        &normalized,
        &[
            "look online",
            "search online",
            "search the web",
            "browse the web",
            "browse online",
            "research online",
            "research current",
        ],
    );
    let current_fact = contains_any(
        &normalized,
        &[
            "latest",
            "most recent",
            "right now",
            "currently",
            "current version",
        ],
    );
    let authoritative_source = contains_any(
        &normalized,
        &[
            "official release notes",
            "official sources",
            "official pages",
            "primary sources",
        ],
    );
    let dependent_follow_up = contains_any(
        &normalized,
        &[
            "then check",
            "exact version",
            "release notes for that",
            "compare the sources",
            "cross-check",
        ],
    );

    external_retrieval && current_fact && authoritative_source && dependent_follow_up
}

pub(super) fn apply_semantic_floor(
    mut assessment: SemanticAssessment,
    prompt: &str,
) -> SemanticAssessment {
    if assessment.demand == SemanticDemand::Advanced {
        return assessment;
    }

    let normalized = prompt.to_lowercase();
    let jurisdiction_count = ["gdpr", "cpra", "lgpd", "pdpa"]
        .iter()
        .filter(|term| normalized.contains(**term))
        .count();
    let legal_reconciliation = jurisdiction_count >= 2
        && contains_any(&normalized, &["reconcile", "conflict", "remediation"])
        && contains_any(
            &normalized,
            &["retention", "consent", "residency", "breach-notification"],
        );
    let current_source_synthesis = normalized.contains("research")
        && normalized.contains("current")
        && contains_any(&normalized, &["primary", "official"])
        && contains_any(
            &normalized,
            &[
                "comparison",
                "compare",
                "synthesis",
                "implies",
                "implications",
            ],
        );
    let interacting_constraints = contains_any(
        &normalized,
        &[
            "recovery plan",
            "critical path",
            "minimizes completion time",
        ],
    ) && normalized.contains("dependencies")
        && normalized.contains("capacity")
        && contains_any(
            &normalized,
            &["contingency", "business hours", "validation precede"],
        );

    let (capability, reason) = if legal_reconciliation {
        (
            SemanticCapability::LegalCompliance,
            SemanticReason::HighStakesJudgment,
        )
    } else if current_source_synthesis {
        (
            SemanticCapability::ResearchSynthesis,
            SemanticReason::SourceSynthesis,
        )
    } else if interacting_constraints {
        (
            SemanticCapability::MultiConstraintReasoning,
            SemanticReason::CrossConstraintAnalysis,
        )
    } else {
        return assessment;
    };

    assessment.demand = SemanticDemand::Advanced;
    assessment.capability = capability;
    assessment.reason = reason;
    assessment.source = format!("{}+{}", assessment.source, OBJECTIVE_SEMANTIC_FLOOR_VERSION);
    assessment
}

pub(super) fn apply_semantic_policy(
    mut assessment: SemanticAssessment,
    prompt: &str,
) -> SemanticAssessment {
    let normalized = prompt.to_ascii_lowercase();
    let exact_rewrite = deterministic_bounded_rewrite_applies(prompt);
    let chat_scoped_recall = (normalized.contains("this chat only")
        && normalized.contains("remember")
        && normalized.contains("reply"))
        || (normalized.contains("return those")
            && normalized.contains("alphabetically")
            && normalized.contains("one per line"));
    let bounded_file_summary = normalized.contains("summarize only the stated facts")
        && normalized.contains("exactly three bullets")
        && normalized.contains("do not use the internet");
    if exact_rewrite || chat_scoped_recall || bounded_file_summary {
        assessment.demand = SemanticDemand::Routine;
        assessment.capability = SemanticCapability::General;
        assessment.confidence = SemanticConfidence::Confident;
        assessment.reason = SemanticReason::BoundedTransformation;
        assessment.source = format!("{}+{BOUNDED_LOCAL_POLICY_VERSION}", assessment.source);
        return assessment;
    }
    apply_semantic_floor(assessment, prompt)
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}
