use regex::Regex;
use std::sync::OnceLock;

pub(crate) fn is_explicit_internal_memory_mutation(user_message: &str) -> bool {
    if super::is_explicit_external_apple_app_mutation(user_message)
        || is_explicit_session_only_memory_request(user_message)
    {
        return false;
    }
    if super::preferred_user_display_name(user_message).is_some() {
        return true;
    }
    let normalized = user_message
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    if normalized.is_empty() || normalized.contains("password") {
        return false;
    }

    static DIRECTIVE: OnceLock<Regex> = OnceLock::new();
    DIRECTIVE
        .get_or_init(|| {
            Regex::new(
                r"(?i)^(?:(?:please\s+)|(?:(?:can|could|would|will)\s+you\s+(?:please\s+)?)|(?:i\s+(?:want|need|would\s+like)\s+you\s+to\s+))?(?:remember|memorize|memorise)\s+\S|^(?:(?:please\s+)|(?:(?:can|could|would|will)\s+you\s+(?:please\s+)?))?(?:make|take)\s+(?:a\s+)?note\s+of\s+\S|^(?:(?:please\s+)|(?:(?:can|could|would|will)\s+you\s+(?:please\s+)?))?(?:add|create|make|save|store|put|keep|record|write)\b[\s\S]{1,200}\b(?:your(?:\s+oomu(?:'s)?)?|agent(?:'s)?|oomu(?:'s)?)\s+(?:long[-\s]?term\s+)?memor(?:y|ies)\b|^(?:please\s+)?(?:update|change|save)\b[\s\S]{1,120}\b(?:user|oomu)\s+profile\b",
            )
            .expect("explicit internal memory mutation regex is valid")
        })
        .is_match(&normalized)
}

pub(super) fn is_explicit_session_only_memory_request(user_message: &str) -> bool {
    let normalized = user_message
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }

    static SESSION_BOUNDARY: OnceLock<Regex> = OnceLock::new();
    SESSION_BOUNDARY
        .get_or_init(|| {
            Regex::new(
                r"(?i)\b(?:for|in|during)\s+(?:this|the\s+current)\s+(?:chat|conversation|session)\s+only\b|\bonly\s+(?:for|in|during)\s+(?:this|the\s+current)\s+(?:chat|conversation|session)\b|\btemporar(?:y|ily)\b[\s\S]{0,80}\b(?:this|the\s+current)\s+(?:chat|conversation|session)\b",
            )
            .expect("session-only memory boundary regex is valid")
        })
        .is_match(&normalized)
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::is_explicit_session_only_memory_request;
    use std::env;

    #[test]
    fn request_never_enters_durable_memory_or_profile() {
        let request = concat!(
            "Remember these temporary test values for this chat only: ",
            "cedar 14, indigo 22, quartz 31. Reply stored."
        );
        assert!(is_explicit_session_only_memory_request(request));
        assert!(!is_explicit_internal_memory_mutation(request));

        let root = env::temp_dir().join(format!(
            "oomu-session-only-memory-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        fs::create_dir_all(&root).expect("temp memory ledger root is created");
        let ledger = MemoryLedger::initialize_at(root.join("oomu_ops.sqlite"))
            .expect("memory ledger initializes");
        let identity = SovereignIdentity::initialize_ephemeral();
        let captured = ledger
            .capture_chat_memories_sync(
                CaptureChatMemoriesRequest {
                    agent_id: "oomu".to_string(),
                    display_name: "OOMU".to_string(),
                    role: "Workstation AI".to_string(),
                    description: "Test agent".to_string(),
                    session_id: "session-ephemeral".to_string(),
                    user_message: request.to_string(),
                    assistant_message: "stored".to_string(),
                    project_id: None,
                },
                &identity,
            )
            .expect("session-only capture is a successful no-op");

        assert!(captured.is_empty());
        assert!(ledger
            .select_user_personality_profile_sync(&identity)
            .expect("profile lookup succeeds")
            .is_none());

        drop(ledger);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn legacy_session_only_entry_is_omitted_from_future_chat_context() {
        let root = env::temp_dir().join(format!(
            "oomu-legacy-session-memory-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        fs::create_dir_all(&root).expect("temp memory ledger root is created");
        let ledger = MemoryLedger::initialize_at(root.join("oomu_ops.sqlite"))
            .expect("memory ledger initializes");
        let identity = SovereignIdentity::initialize_ephemeral();
        let legacy_content = concat!(
            "Remember these temporary test values for this chat only: ",
            "cedar 14, indigo 22, quartz 31."
        );
        ledger
            .upsert_agent_memory_sync(
                "oomu",
                "user_profile",
                "identity_or_goal",
                legacy_content,
                0.9,
                "legacy-session",
                "private",
                &identity,
            )
            .expect("legacy signed entry is created");

        let context = ledger
            .hydrate_agent_context_sync_with_memory_limit(
                HydrateAgentContextRequest {
                    agent_id: "oomu".to_string(),
                    display_name: "OOMU".to_string(),
                    role: "Workstation AI".to_string(),
                    description: "Test agent".to_string(),
                    system_prompt: "Answer the user.".to_string(),
                    latest_message: "What should we discuss?".to_string(),
                    provider_id: Some("local_model".to_string()),
                    model_id: Some("gemma-4".to_string()),
                    tool_registry_offline: false,
                    background_mod_event: false,
                    layout_schema: None,
                    project_id: None,
                    verified_filesystem_context: None,
                },
                10,
                &identity,
            )
            .expect("future context hydrates");

        assert!(context.memories.is_empty());
        assert!(!context.prompt_context.contains("cedar 14"));
        drop(ledger);
        let _ = fs::remove_dir_all(root);
    }
}
