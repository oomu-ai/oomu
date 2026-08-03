use crate::db::{ChatMessageRecord, PersistenceEngine, PreparedContextualFileAction};
use crate::sovereign_identity::SovereignIdentity;
use regex::Regex;
use std::{fs, path::Path};

pub(super) fn resolve_turn_objective(user_objective: Option<&str>, prompt: &str) -> String {
    match user_objective {
        Some(objective) => objective.trim().to_string(),
        None => legacy_turn_objective(prompt).to_string(),
    }
}

fn legacy_turn_objective(prompt: &str) -> &str {
    let candidate = super::routing_intent_latest_turn(prompt)
        .unwrap_or(prompt)
        .trim();
    let before_support = [
        "\nLocal text attachment:",
        "\nAttachment context:",
        "\nSupporting content:",
    ]
    .iter()
    .filter_map(|marker| candidate.find(marker))
    .min()
    .map(|index| &candidate[..index])
    .unwrap_or(candidate);
    before_support
        .split_once("\n\n")
        .map(|(objective, _)| objective)
        .unwrap_or(before_support)
        .trim()
}

pub(super) fn resolve_persisted_objective_for_turn(
    objective: String,
    session_id: Option<&str>,
    persistence: &PersistenceEngine,
) -> Result<String, super::AgenticLoopError> {
    let Some(session_id) = session_id.filter(|session_id| !session_id.trim().is_empty()) else {
        return Ok(objective);
    };
    resolve_persisted_markdown_objective(&objective, session_id, persistence).map_err(|message| {
        super::AgenticLoopError {
            code: "contextual_file_preparation_failed",
            boundary: "AgentPlanning",
            message,
            mlc_path: None,
        }
    })
}

pub(super) fn resolve_persisted_markdown_objective(
    objective: &str,
    session_id: &str,
    persistence: &PersistenceEngine,
) -> Result<String, String> {
    let objective_directory = explicit_destination_directory(objective);
    let approved_file_marker = super::contains_approved_file_marker(objective);
    if objective_directory.is_none() && !approved_file_marker {
        return Ok(objective.to_string());
    }
    if is_agent_owned_markdown_creation_request(objective) {
        return Ok(objective.to_string());
    }

    let messages = persistence
        .select_chat_messages(session_id)
        .map_err(|error| error.to_string())?;
    let Some(current_message_index) = messages.iter().rposition(|message| message.role == "user")
    else {
        return Ok(objective.to_string());
    };
    let persisted_current = messages[current_message_index].content.trim();
    let directory = match objective_directory {
        Some(directory) if persisted_current == objective.trim() => directory,
        None if approved_file_marker => {
            let Some(directory) = explicit_destination_directory(persisted_current) else {
                return Ok(objective.to_string());
            };
            directory
        }
        _ => return Ok(objective.to_string()),
    };
    let Some(prior_objective) = messages[..current_message_index]
        .iter()
        .rev()
        .find(|message| message.role == "user")
        .filter(|message| is_agent_owned_markdown_creation_request(&message.content))
        .map(|message| message.content.trim())
    else {
        return Ok(objective.to_string());
    };

    Ok(format!(
        "{prior_objective}\n\nUse this destination directory supplied in the current turn: {directory}"
    ))
}

pub(super) fn is_directory_only_markdown_request(objective: &str) -> bool {
    is_agent_owned_markdown_creation_request(objective)
        && explicit_destination_directory(objective).is_some()
}

pub(super) fn prepare_markdown_action(
    objective: &str,
    session_id: &str,
    persistence: &PersistenceEngine,
) -> Result<Option<PreparedContextualFileAction>, String> {
    if !is_directory_only_markdown_request(objective)
        || !super::plan_coverage::objective_output_file_references(objective).is_empty()
    {
        return Ok(None);
    }
    let requested_directory = explicit_destination_directory(objective)
        .ok_or_else(|| "The requested report folder was not clear.".to_string())?;
    let requested = Path::new(&requested_directory);
    let metadata = fs::symlink_metadata(requested)
        .map_err(|_| "The requested report folder does not exist.".to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("The requested report destination must be an existing folder.".to_string());
    }
    let directory = fs::canonicalize(requested)
        .map_err(|_| "The requested report folder could not be verified.".to_string())?;
    let messages = persistence
        .select_chat_messages(session_id)
        .map_err(|error| error.to_string())?;
    let content = conversation_markdown_report(objective, &messages)?;
    let filename =
        collision_safe_markdown_filename(&directory, proposed_markdown_filename(objective))?;
    let destination = directory.join(&filename);
    let content_message_id = messages
        .iter()
        .rev()
        .find(|message| message.role == "assistant")
        .or_else(|| messages.last())
        .map(|message| message.id)
        .unwrap_or_default();
    Ok(Some(PreparedContextualFileAction {
        directory_path: directory.to_string_lossy().to_string(),
        destination_path: destination.to_string_lossy().to_string(),
        filename,
        requested_format: "md".to_string(),
        content_message_id,
        content_digest: crate::foundation::digest::sha256_hex(content.as_bytes()),
        content,
    }))
}

pub(super) fn prepare_contextual_action(
    objective: &str,
    session_id: &str,
    persistence: &PersistenceEngine,
    identity: &SovereignIdentity,
) -> Result<Option<crate::db::ContextualFileActionPreparation>, String> {
    if let Some(preparation) = prepare_markdown_action(objective, session_id, persistence)? {
        return Ok(Some(crate::db::ContextualFileActionPreparation::Ready(
            preparation,
        )));
    }
    persistence.prepare_contextual_file_action(
        session_id,
        objective,
        super::contextual_route::is_contextual_mutation_request(objective),
        identity,
    )
}

fn explicit_destination_directory(objective: &str) -> Option<String> {
    let normalized = objective.to_ascii_lowercase();
    let desktop_pattern = Regex::new(
        r"(?i)\b(?:my|the)\s+desktop(?:\s+(?:folder|directory))?\b|\bdesktop\s+(?:folder|directory)\b",
    )
    .expect("desktop destination regex is valid");
    let destination_cue = [
        " to ",
        " in ",
        " into ",
        " inside ",
        " on ",
        "destination",
        "path",
        "folder",
        "directory",
    ]
    .iter()
    .any(|cue| normalized.contains(cue));

    let mut candidates = Vec::new();
    let absolute_pattern =
        Regex::new(r#"(?:^|[\s:'"`])(/[^\n\r'"`]+)"#).expect("absolute destination regex is valid");
    for capture in absolute_pattern
        .captures_iter(objective)
        .filter_map(|captures| captures.get(1))
    {
        let raw = capture
            .as_str()
            .trim()
            .trim_end_matches(|character: char| matches!(character, '.' | ',' | ';' | '!' | '?'))
            .replace("\\ ", " ")
            .replace("\\~", "~");
        if !Path::new(&raw).is_absolute() {
            continue;
        }
        candidates.push(raw.clone());

        // A directory can be followed by prose in a natural-language turn. Prefer the
        // longest existing directory prefix without ever interpreting a file as a folder.
        let mut prefix = raw.as_str();
        while let Some((head, _)) = prefix.rsplit_once(char::is_whitespace) {
            prefix = head.trim_end();
            if Path::new(prefix).is_absolute()
                && fs::symlink_metadata(prefix)
                    .ok()
                    .is_some_and(|metadata| !metadata.file_type().is_symlink() && metadata.is_dir())
            {
                candidates.push(prefix.to_string());
                break;
            }
        }
    }
    if candidates.is_empty() && destination_cue && desktop_pattern.is_match(objective) {
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join("Desktop").to_string_lossy().to_string());
        }
    }
    let existing = candidates
        .iter()
        .filter(|candidate| {
            fs::symlink_metadata(candidate)
                .ok()
                .is_some_and(|metadata| !metadata.file_type().is_symlink() && metadata.is_dir())
        })
        .cloned()
        .collect::<Vec<_>>();
    if !existing.is_empty() {
        candidates = existing;
    }
    candidates.sort();
    candidates.dedup();
    (candidates.len() == 1).then(|| candidates.remove(0))
}

fn is_agent_owned_markdown_creation_request(objective: &str) -> bool {
    let normalized = objective.to_ascii_lowercase();
    let markdown = normalized.contains("markdown") || normalized.contains(".md");
    let creation = Regex::new(r"(?i)\b(?:write|create|save|put|export|make|generate|produce)\b")
        .expect("agent-owned Markdown creation regex is valid")
        .is_match(objective);
    markdown
        && creation
        && super::plan_coverage::objective_output_file_references(objective).is_empty()
}

fn proposed_markdown_filename(objective: &str) -> &'static str {
    let normalized = objective.to_ascii_lowercase();
    if normalized.contains("diagnostic") {
        "oomu-test-diagnostic-report.md"
    } else if normalized.contains("search") || normalized.contains("research") {
        "oomu-search-report.md"
    } else if normalized.contains("test") {
        "oomu-test.md"
    } else {
        "oomu-conversation-report.md"
    }
}

fn collision_safe_markdown_filename(directory: &Path, proposed: &str) -> Result<String, String> {
    let basename = Path::new(proposed)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("oomu-report")
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if basename.is_empty() || basename.len() > 96 {
        return Err("OOMU could not choose a safe report filename.".to_string());
    }
    for suffix in 1..=10_000usize {
        let filename = if suffix == 1 {
            format!("{basename}.md")
        } else {
            format!("{basename}-{suffix}.md")
        };
        let candidate = directory.join(&filename);
        if !candidate.exists() && candidate.parent() == Some(directory) {
            return Ok(filename);
        }
    }
    Err("OOMU could not find an unused report filename in that folder.".to_string())
}

fn conversation_markdown_report(
    objective: &str,
    messages: &[ChatMessageRecord],
) -> Result<String, String> {
    let normalized = objective.to_ascii_lowercase();
    let title = if normalized.contains("diagnostic") {
        "OOMU test diagnostic report"
    } else if normalized.contains("search") || normalized.contains("research") {
        "OOMU search report"
    } else if normalized.contains("test") {
        "OOMU test"
    } else {
        "OOMU conversation report"
    };
    let mut report = format!(
        "# {title}\n\nThis report was composed from the verified conversation record.\n\n## Requested purpose\n\n"
    );
    report.push_str(objective.trim());
    report.push_str("\n\n## Conversation evidence\n");
    let mut included = 0usize;
    for message in messages.iter().filter(|message| {
        matches!(message.role.as_str(), "user" | "assistant")
            && !message.content.trim().is_empty()
            && (message.role != "assistant"
                || !contains_unresolved_template_scaffold(&message.content))
    }) {
        included += 1;
        let role = if message.role == "user" {
            "User"
        } else {
            "OOMU"
        };
        report.push_str(&format!(
            "\n### {role} turn {included}\n\n{}\n",
            message.content.trim()
        ));
    }
    if included == 0 {
        return Err("The current conversation does not contain reportable content.".to_string());
    }
    let receipts = messages
        .iter()
        .filter_map(|message| message.metadata_json.as_deref())
        .filter_map(|metadata| serde_json::from_str::<serde_json::Value>(metadata).ok())
        .filter(|metadata| {
            metadata
                .get("checkpointKind")
                .and_then(serde_json::Value::as_str)
                == Some("verified_sovereign_search")
        })
        .collect::<Vec<_>>();
    if !receipts.is_empty() {
        report.push_str("\n## Verified native search receipts\n");
        for receipt in receipts {
            let index = receipt
                .get("searchInvocationIndex")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default();
            let engine = receipt
                .get("searchEngine")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            let count = receipt
                .get("resultCount")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default();
            report.push_str(&format!(
                "\n- Search {index}: engine {engine}, {count} verified result(s)."
            ));
            if let Some(urls) = receipt
                .get("sourceUrls")
                .and_then(serde_json::Value::as_array)
            {
                for url in urls.iter().filter_map(serde_json::Value::as_str) {
                    report.push_str(&format!("\n  - {url}"));
                }
            }
        }
        report.push('\n');
    }
    Ok(report)
}

fn contains_unresolved_template_scaffold(content: &str) -> bool {
    let normalized = content.to_ascii_lowercase();
    let unresolved_bracket = Regex::new(r"(?i)\[(?:insert|your|replace|todo)\b[^\]\r\n]*\]")
        .expect("unresolved template scaffold regex is valid")
        .is_match(content);
    unresolved_bracket
        || normalized.contains("yyyy-mm-dd")
        || normalized.contains("placeholder for contextual data")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_is_conversation_derived_and_collision_safe() {
        let root = std::env::temp_dir().join(format!(
            "oomu-agent-owned-report-{}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        let output = root.join("reports");
        std::fs::create_dir_all(&output).unwrap();
        let persistence = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let session = persistence
            .ensure_chat_session(crate::db::CreateChatSessionRequest {
                agent_id: "agent-report".to_string(),
                provider_id: "local_model".to_string(),
                model_id: "gemma-test".to_string(),
                title: Some("Sprint 294 report".to_string()),
                dynamic_routing_override: None,
                workspace_id: None,
            })
            .unwrap();
        persistence
            .insert_chat_message(
                &session.id,
                &session.agent_id,
                "user",
                "Go online and research the Kimi and Fable accusation",
            )
            .unwrap();
        persistence
            .insert_chat_message(
                &session.id,
                &session.agent_id,
                "assistant",
                "The public sources describe the accusation and response.",
            )
            .unwrap();
        persistence.insert_chat_message_with_metadata(
            &session.id, &session.agent_id, "system", "Verified native search receipt", None, None,
            Some(&serde_json::json!({"checkpointKind":"verified_sovereign_search","searchInvocationIndex":1,"searchEngine":"duckduckgo_html","resultCount":2,"sourceUrls":["https://example.com/official-a","https://example.com/official-b"]})),
        ).unwrap();
        let objective = format!(
            "Create a Markdown diagnostic report of what happened in this test and put it in {}.",
            output.to_string_lossy()
        );
        let first = prepare_markdown_action(&objective, &session.id, &persistence)
            .unwrap()
            .unwrap();
        assert_eq!(first.filename, "oomu-test-diagnostic-report.md");
        assert!(first.content.contains("Go online and research"));
        assert!(first.content.contains("Verified native search receipts"));
        assert!(first.content.contains("https://example.com/official-a"));
        std::fs::write(&first.destination_path, first.content.as_bytes()).unwrap();
        let second = prepare_markdown_action(&objective, &session.id, &persistence)
            .unwrap()
            .unwrap();
        assert_eq!(second.filename, "oomu-test-diagnostic-report-2.md");
        assert_eq!(
            std::fs::read_to_string(&first.destination_path).unwrap(),
            first.content
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn generic_test_markdown_uses_a_safe_agent_owned_name() {
        let root = std::env::temp_dir().join(format!(
            "oomu-agent-owned-test-{}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        let output = root.join("reports");
        std::fs::create_dir_all(&output).unwrap();
        let persistence = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let session = persistence
            .ensure_chat_session(crate::db::CreateChatSessionRequest {
                agent_id: "agent-test-file".to_string(),
                provider_id: "local_model".to_string(),
                model_id: "gemma-test".to_string(),
                title: Some("Agent-owned test file".to_string()),
                dynamic_routing_override: None,
                workspace_id: None,
            })
            .unwrap();
        let objective = format!(
            "Write a test Markdown file to {} and choose the file name and content.",
            output.to_string_lossy()
        );
        persistence
            .insert_chat_message(&session.id, &session.agent_id, "user", &objective)
            .unwrap();

        let first = prepare_markdown_action(&objective, &session.id, &persistence)
            .unwrap()
            .unwrap();
        assert_eq!(first.filename, "oomu-test.md");
        assert!(first.content.contains("choose the file name and content"));
        assert!(!first.content.trim().is_empty());
        std::fs::write(&first.destination_path, first.content.as_bytes()).unwrap();
        let second = prepare_markdown_action(&objective, &session.id, &persistence)
            .unwrap()
            .unwrap();
        assert_eq!(second.filename, "oomu-test-2.md");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unresolved_model_template_scaffolds_never_enter_the_created_report() {
        let root = std::env::temp_dir().join(format!(
            "oomu-agent-owned-template-filter-{}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        let output = root.join("reports");
        std::fs::create_dir_all(&output).unwrap();
        let persistence = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let session = persistence
            .ensure_chat_session(crate::db::CreateChatSessionRequest {
                agent_id: "agent-template-filter".to_string(),
                provider_id: "local_model".to_string(),
                model_id: "gemma-test".to_string(),
                title: Some("Template scaffold filter".to_string()),
                dynamic_routing_override: None,
                workspace_id: None,
            })
            .unwrap();
        let objective = format!(
            "Write a test Markdown file in {} and choose the filename.",
            output.to_string_lossy()
        );
        persistence
            .insert_chat_message(&session.id, &session.agent_id, "user", &objective)
            .unwrap();
        persistence
            .insert_chat_message(
                &session.id,
                &session.agent_id,
                "assistant",
                "Context ID: [Insert Unique Context Identifier Here]\nCreated: YYYY-MM-DD HH:MM:SS",
            )
            .unwrap();

        let prepared = prepare_markdown_action(&objective, &session.id, &persistence)
            .unwrap()
            .unwrap();
        assert!(prepared.content.contains(&objective));
        assert!(!prepared.content.contains("[Insert Unique"));
        assert!(!prepared.content.contains("YYYY-MM-DD"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn path_only_follow_up_reconstructs_the_persisted_file_action() {
        let root = std::env::temp_dir().join(format!(
            "oomu-agent-owned-continuation-{}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        let output = root.join("chosen destination");
        std::fs::create_dir_all(&output).unwrap();
        let persistence = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let session = persistence
            .ensure_chat_session(crate::db::CreateChatSessionRequest {
                agent_id: "agent-continuation".to_string(),
                provider_id: "local_model".to_string(),
                model_id: "gemma-test".to_string(),
                title: Some("Persisted action continuation".to_string()),
                dynamic_routing_override: None,
                workspace_id: None,
            })
            .unwrap();
        let original = "Write a test Markdown file and choose a suitable filename and content.";
        let follow_up = output.to_string_lossy().to_string();
        persistence
            .insert_chat_message(&session.id, &session.agent_id, "user", original)
            .unwrap();
        persistence
            .insert_chat_message(
                &session.id,
                &session.agent_id,
                "system",
                "A path is needed.",
            )
            .unwrap();
        persistence
            .insert_chat_message(&session.id, &session.agent_id, "user", &follow_up)
            .unwrap();

        let resolved =
            resolve_persisted_markdown_objective(&follow_up, &session.id, &persistence).unwrap();
        assert!(resolved.contains(original));
        assert!(resolved.contains(&follow_up));
        let prepared = prepare_markdown_action(&resolved, &session.id, &persistence)
            .unwrap()
            .unwrap();
        assert_eq!(prepared.filename, "oomu-test.md");
        assert_eq!(
            prepared.directory_path,
            fs::canonicalize(&output).unwrap().to_string_lossy()
        );
        assert!(prepared.content.contains(original));
        assert!(prepared.content.contains(&follow_up));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn approved_directory_marker_reconstructs_the_accepted_path_follow_up() {
        let root = std::env::temp_dir().join(format!(
            "oomu-agent-owned-approved-directory-{}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        let output = root.join("approved destination");
        std::fs::create_dir_all(&output).unwrap();
        let persistence = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let session = persistence
            .ensure_chat_session(crate::db::CreateChatSessionRequest {
                agent_id: "agent-approved-directory".to_string(),
                provider_id: "local_model".to_string(),
                model_id: "gemma-test".to_string(),
                title: Some("Approved directory continuation".to_string()),
                dynamic_routing_override: None,
                workspace_id: None,
            })
            .unwrap();
        let original = "Write a test Markdown file and choose a suitable filename and content.";
        persistence
            .insert_chat_message(&session.id, &session.agent_id, "user", original)
            .unwrap();
        persistence
            .insert_chat_message(
                &session.id,
                &session.agent_id,
                "assistant",
                "I will choose the filename after you provide the destination.",
            )
            .unwrap();
        persistence
            .insert_chat_message(
                &session.id,
                &session.agent_id,
                "user",
                &output.to_string_lossy(),
            )
            .unwrap();

        let resolved =
            resolve_persisted_markdown_objective("[approved file]", &session.id, &persistence)
                .unwrap();
        assert!(resolved.contains(original));
        assert!(resolved.contains(&output.to_string_lossy().to_string()));
        super::super::validate_agent_planner_objective(&resolved)
            .expect("the reconstructed continuation is an executable file action");
        let prepared = prepare_markdown_action(&resolved, &session.id, &persistence)
            .unwrap()
            .unwrap();
        assert_eq!(prepared.filename, "oomu-test.md");
        assert!(prepared.content.contains("I will choose the filename"));
        let destination = prepared.destination_path.clone();
        let content_message_reference =
            format!("assistant message {}", prepared.content_message_id);
        let content_digest = prepared.content_digest.clone();
        let draft = super::super::contextual_route::deterministic_contextual_file_draft(prepared);
        assert!(draft.steps[0].step.contains(&destination));
        assert!(!draft.steps[0].step.contains(&content_message_reference));
        assert!(!draft.steps[0].step.contains(&content_digest));
        let executable =
            super::super::plan_coverage::prepare_draft_for_execution(&resolved, draft, false)
                .unwrap();
        assert_eq!(executable.steps.len(), 1);
        let route = super::super::ModelRouteDecision {
            selected_model: super::super::ModelMetadata::local_gemma(),
            provider_config_id: None,
            provider_id: Some("local_model".to_string()),
            recommended_model: None,
            requires_principal_authorization: false,
            reason: "Deterministic contextual file plan".to_string(),
            context_excerpt_count: 0,
            context_sources: Vec::new(),
        };
        let plan = super::super::generated_draft_to_plan(
            resolved,
            executable,
            route,
            super::super::ContextBundle {
                excerpts: Vec::new(),
                claim_sources: Vec::new(),
                inherited_artifact_hashes: Vec::new(),
            },
        );
        let identity = SovereignIdentity::initialize_ephemeral();
        let signed = super::super::sign_plan(plan, &identity).unwrap();
        crate::tools::create_file_contract::register_preview_contract();
        crate::verifier::MlcVerifier::new()
            .verify_plan_preview(&signed, &identity)
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn path_only_follow_up_does_not_resume_a_stale_markdown_topic() {
        let root = std::env::temp_dir().join(format!(
            "oomu-agent-owned-stale-continuation-{}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        let output = root.join("destination");
        std::fs::create_dir_all(&output).unwrap();
        let persistence = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let session = persistence
            .ensure_chat_session(crate::db::CreateChatSessionRequest {
                agent_id: "agent-stale-continuation".to_string(),
                provider_id: "local_model".to_string(),
                model_id: "gemma-test".to_string(),
                title: Some("Stale continuation protection".to_string()),
                dynamic_routing_override: None,
                workspace_id: None,
            })
            .unwrap();
        persistence
            .insert_chat_message(
                &session.id,
                &session.agent_id,
                "user",
                "Write a test Markdown file and choose its filename.",
            )
            .unwrap();
        persistence
            .insert_chat_message(
                &session.id,
                &session.agent_id,
                "user",
                "Instead, tell me about the weather.",
            )
            .unwrap();
        let follow_up = output.to_string_lossy().to_string();
        persistence
            .insert_chat_message(&session.id, &session.agent_id, "user", &follow_up)
            .unwrap();

        assert_eq!(
            resolve_persisted_markdown_objective(&follow_up, &session.id, &persistence).unwrap(),
            follow_up
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unmatched_current_turn_cannot_recover_from_chat_history() {
        let root = std::env::temp_dir().join(format!(
            "oomu-agent-owned-unmatched-continuation-{}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        let output = root.join("destination");
        std::fs::create_dir_all(&output).unwrap();
        let persistence = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let session = persistence
            .ensure_chat_session(crate::db::CreateChatSessionRequest {
                agent_id: "agent-unmatched-continuation".to_string(),
                provider_id: "local_model".to_string(),
                model_id: "gemma-test".to_string(),
                title: Some("Unmatched continuation protection".to_string()),
                dynamic_routing_override: None,
                workspace_id: None,
            })
            .unwrap();
        persistence
            .insert_chat_message(
                &session.id,
                &session.agent_id,
                "user",
                "Write a test Markdown file and choose its filename.",
            )
            .unwrap();
        let unmatched = output.to_string_lossy().to_string();

        assert_eq!(
            resolve_persisted_markdown_objective(&unmatched, &session.id, &persistence).unwrap(),
            unmatched
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn creation_verbs_require_word_boundaries() {
        let root = std::env::temp_dir().join(format!(
            "oomu-agent-owned-word-boundary-{}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        std::fs::create_dir_all(&root).unwrap();
        assert!(!is_agent_owned_markdown_creation_request(&format!(
            "Do not overwrite the existing Markdown file in {}.",
            root.to_string_lossy()
        )));
        assert!(is_agent_owned_markdown_creation_request(&format!(
            "Write a Markdown file in {}.",
            root.to_string_lossy()
        )));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn desktop_folder_language_resolves_against_the_real_home_directory() {
        let expected = dirs::home_dir()
            .expect("the test host has a home directory")
            .join("Desktop")
            .to_string_lossy()
            .to_string();
        assert_eq!(
            explicit_destination_directory(
                "Can you write a test Markdown file to my Desktop folder now?"
            ),
            Some(expected)
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_destination_remains_rejected() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "oomu-agent-owned-symlink-{}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        let output = root.join("real");
        let link = root.join("linked");
        std::fs::create_dir_all(&output).unwrap();
        symlink(&output, &link).unwrap();
        let persistence = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let session = persistence
            .ensure_chat_session(crate::db::CreateChatSessionRequest {
                agent_id: "agent-symlink".to_string(),
                provider_id: "local_model".to_string(),
                model_id: "gemma-test".to_string(),
                title: Some("Reject symlink destination".to_string()),
                dynamic_routing_override: None,
                workspace_id: None,
            })
            .unwrap();
        let objective = format!("Write a Markdown file into {}.", link.to_string_lossy());
        persistence
            .insert_chat_message(&session.id, &session.agent_id, "user", &objective)
            .unwrap();

        let error = prepare_markdown_action(&objective, &session.id, &persistence).unwrap_err();
        assert_eq!(
            error,
            "The requested report destination must be an existing folder."
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
