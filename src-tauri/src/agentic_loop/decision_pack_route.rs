use super::{plan_coverage, ChatIntentRoute, ChatIntentRouteDecision};

pub(super) fn classify(prompt: &str) -> Option<ChatIntentRouteDecision> {
    plan_coverage::requests_evidence_bound_decision_pack(prompt).then(|| {
        ChatIntentRouteDecision {
            route: ChatIntentRoute::AgenticPlanner,
            requires_local_access: true,
            decision_source: "deterministic_decision_pack_filter".to_string(),
            reason: "An explicit local path, attached file, or typed filename paired with a file operation requires the approval-gated planner."
                .to_string(),
            matched_signals: vec!["evidence-bound decision pack".to_string()],
            status_label: "OOMU is planning local actions...".to_string(),
        }
    })
}
