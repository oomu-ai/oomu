#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrivateAppleReadKind {
    Calendar,
    Mail,
    Notes,
    Reminders,
    Contacts,
    Messages,
    Photos,
    Music,
}

impl PrivateAppleReadKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Calendar => "calendar",
            Self::Mail => "mail",
            Self::Notes => "notes",
            Self::Reminders => "reminders",
            Self::Contacts => "contacts",
            Self::Messages => "messages",
            Self::Photos => "photos",
            Self::Music => "music",
        }
    }
}

pub(crate) fn detect_from_objective(objective: &str) -> Option<PrivateAppleReadKind> {
    let normalized = objective.trim().to_ascii_lowercase();
    if normalized.is_empty()
        || contains_mutation(&normalized)
        || !contains_read_intent(&normalized)
        || !contains_private_scope(&normalized)
    {
        return None;
    }
    [
        (
            PrivateAppleReadKind::Calendar,
            &["calendar", "schedule", "events"] as &[_],
        ),
        (
            PrivateAppleReadKind::Mail,
            &["mail", "email", "emails", "inbox"],
        ),
        (PrivateAppleReadKind::Notes, &["note", "notes"]),
        (PrivateAppleReadKind::Reminders, &["reminder", "reminders"]),
        (
            PrivateAppleReadKind::Contacts,
            &["contact", "contacts", "address book"],
        ),
        (
            PrivateAppleReadKind::Messages,
            &["message", "messages", "imessage"],
        ),
        (
            PrivateAppleReadKind::Photos,
            &["photo", "photos", "pictures"],
        ),
        (
            PrivateAppleReadKind::Music,
            &["music", "songs", "albums", "library"],
        ),
    ]
    .into_iter()
    .find_map(|(kind, markers)| {
        markers
            .iter()
            .any(|marker| contains_marker(&normalized, marker))
            .then_some(kind)
    })
}

pub(crate) fn is_bounded_read_objective(objective: &str) -> bool {
    let normalized = objective.trim().to_ascii_lowercase();
    !normalized.is_empty() && contains_read_intent(&normalized) && !contains_mutation(&normalized)
}

pub(in crate::inference) async fn resolve(
    gemma: &GemmaService,
    local_provider_id: &str,
    local_model_id: &str,
    kind: PrivateAppleReadKind,
) -> Result<DynamicModelRouteDecision, InferenceError> {
    resolve_local_model_route_from_assessment(
        local_provider_id,
        local_model_id,
        assessment(gemma, kind),
    )
}

pub(in crate::inference) async fn resolve_frozen(
    gemma: &GemmaService,
    local_provider_id: &str,
    local_model_id: &str,
    kind: PrivateAppleReadKind,
) -> Result<DynamicModelRouteDecision, InferenceError> {
    resolve_local_model_route_from_assessment(
        local_provider_id,
        local_model_id,
        assessment(gemma, kind),
    )
}

pub(super) fn is_policy_source(source: &str) -> bool {
    source.starts_with(POLICY_VERSION)
}

fn assessment(gemma: &GemmaService, kind: PrivateAppleReadKind) -> SemanticAssessment {
    SemanticAssessment {
        demand: SemanticDemand::Routine,
        capability: SemanticCapability::General,
        confidence: SemanticConfidence::Confident,
        reason: SemanticReason::BoundedTransformation,
        source: format!("{POLICY_VERSION}:{}", kind.as_str()),
        classifier_latency_ms: 0,
        classifier_model_id: None,
        readiness_generation: gemma.classifier_health().readiness_generation,
        recovery_attempted: false,
    }
}

fn contains_read_intent(normalized: &str) -> bool {
    [
        "what's",
        "what is",
        "show",
        "read",
        "check",
        "list",
        "summarize",
        "summary",
        "tell me",
        "do i have",
        "find",
        "recent",
        "today",
        "tomorrow",
        "unread",
    ]
    .iter()
    .any(|marker| contains_marker(normalized, marker))
}

fn contains_private_scope(normalized: &str) -> bool {
    [
        "my",
        "our",
        "do i have",
        "did i",
        "have i",
        "for me",
        "mine",
    ]
    .iter()
    .any(|marker| contains_marker(normalized, marker))
}

fn contains_mutation(normalized: &str) -> bool {
    [
        "create",
        "add",
        "write",
        "send",
        "delete",
        "remove",
        "move",
        "change",
        "edit",
        "update",
        "reply",
        "forward",
        "invite",
        "schedule a",
        "schedule an",
        "cancel",
        "mark as",
    ]
    .iter()
    .any(|marker| contains_marker(normalized, marker))
}

fn contains_marker(value: &str, marker: &str) -> bool {
    value.match_indices(marker).any(|(start, matched)| {
        let end = start + matched.len();
        is_token_boundary(value[..start].chars().next_back())
            && is_token_boundary(value[end..].chars().next())
    })
}

fn is_token_boundary(character: Option<char>) -> bool {
    character.is_none_or(|value| !value.is_alphanumeric() && value != '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_read_detection_accepts_reads_and_rejects_mutations() {
        assert_eq!(
            detect_from_objective("What's on my calendar today?"),
            Some(PrivateAppleReadKind::Calendar)
        );
        assert_eq!(
            detect_from_objective("Show my schedule today"),
            Some(PrivateAppleReadKind::Calendar)
        );
        assert_eq!(
            detect_from_objective("Do I have any unread emails?"),
            Some(PrivateAppleReadKind::Mail)
        );
        assert_eq!(
            detect_from_objective("Create an event on my calendar"),
            None
        );
        assert_eq!(detect_from_objective("Send that email"), None);
        assert_eq!(
            detect_from_objective("Show my address book"),
            Some(PrivateAppleReadKind::Contacts)
        );
        assert_eq!(
            detect_from_objective("Show my recent email senders"),
            Some(PrivateAppleReadKind::Mail)
        );
        assert_eq!(
            detect_from_objective("List my deleted messages"),
            Some(PrivateAppleReadKind::Messages)
        );
        assert_eq!(detect_from_objective("Tell me about music theory"), None);
        assert_eq!(detect_from_objective("What is the Mail app?"), None);
    }

    #[tokio::test]
    async fn private_apple_read_routes_deterministically_local() {
        let path = std::env::temp_dir().join(format!(
            "oomu-private-apple-route-{}-{}.db",
            std::process::id(),
            crate::foundation::clock::unix_time_ns_u128()
        ));
        let gemma = GemmaService::new_disabled("classifier unavailable by test contract");
        for kind in [
            PrivateAppleReadKind::Calendar,
            PrivateAppleReadKind::Mail,
            PrivateAppleReadKind::Notes,
        ] {
            let route = resolve(
                &gemma,
                "local_model",
                crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID,
                kind,
            )
            .await
            .expect("typed private read stays local without classifier readiness");
            assert_eq!(route.tier, "local_tier_1");
            assert_eq!(route.provider_id, "local_model");
            assert_eq!(route.model_id, crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID);
            assert!(route.classifier_source.starts_with(POLICY_VERSION));
            assert_eq!(route.classifier_latency_ms, 0);
        }
        assert!(
            !path.exists(),
            "private routing never opens provider storage"
        );
    }
}
use super::{
    resolve_local_model_route_from_assessment, DynamicModelRouteDecision, SemanticAssessment,
    SemanticCapability, SemanticConfidence, SemanticDemand, SemanticReason,
};
use crate::{gemma::GemmaService, inference::InferenceError};

pub(crate) const POLICY_VERSION: &str = "private_apple_read_local_v1";
