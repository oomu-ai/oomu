use super::*;
use crate::sovereign_identity::{SignatureBlock, SovereignIdentity};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VerifiedFilesystemContext {
    pub session_id: String,
    pub source_turn_id: String,
    pub operation: String,
    pub canonical_path: String,
    pub target_kind: String,
    pub verified_receipt_digest: String,
    pub completed_at_ms: i64,
    pub result_status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantContentReference {
    pub message_id: i64,
    pub content: String,
    pub content_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedContextualFileAction {
    pub directory_path: String,
    pub destination_path: String,
    pub filename: String,
    pub requested_format: String,
    pub content_message_id: i64,
    pub content_digest: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextualFileActionPreparation {
    NeedsFilename,
    Ready(PreparedContextualFileAction),
}

#[derive(Debug, Clone)]
struct PendingContextualFileAction {
    directory_path: String,
    directory_receipt_digest: String,
    content_message_id: i64,
    content_digest: String,
    requested_format: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct FilesystemReceiptPayload {
    workspace_id: String,
    session_id: String,
    project_id: Option<String>,
    source_turn_id: String,
    source_generation_token: String,
    operation: String,
    canonical_path: String,
    target_kind: String,
    completed_at_ms: i64,
    result_status: String,
}

impl PersistenceEngine {
    pub fn record_verified_filesystem_context(
        &self,
        turn: &ChatTurnPersistenceContext,
        operation: &str,
        path: &str,
        target_kind: &str,
        identity: &SovereignIdentity,
    ) -> Result<VerifiedFilesystemContext, String> {
        if !matches!(operation, "file_read" | "file_list" | "file_write")
            || !matches!(target_kind, "file" | "directory")
        {
            return Err("unsupported verified filesystem context".to_string());
        }
        self.ensure_chat_turn_for_native_action(turn)
            .map_err(|error| error.to_string())?;
        let canonical = fs::canonicalize(path).map_err(|error| error.to_string())?;
        let metadata = fs::metadata(&canonical).map_err(|error| error.to_string())?;
        if (target_kind == "directory") != metadata.is_dir()
            || (target_kind == "file") != metadata.is_file()
        {
            return Err("verified filesystem target kind changed".to_string());
        }
        let canonical_path = canonical.to_string_lossy().to_string();
        let _guard = self.lock_writes();
        let mut connection = self.open_connection().map_err(|error| error.to_string())?;
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        let workspace_id =
            workspace_id_for_chat_session(&transaction, &turn.session_id, &self.workspace_id)
                .map_err(|error| error.to_string())?;
        let project_id = transaction
            .query_row(
                "SELECT project_id FROM chat_sessions WHERE id = ?1 AND workspace_id = ?2",
                params![turn.session_id, workspace_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .map_err(|error| error.to_string())?;
        let completed_at_ms = unix_time_ms();
        let payload = FilesystemReceiptPayload {
            workspace_id: workspace_id.clone(),
            session_id: turn.session_id.clone(),
            project_id: project_id.clone(),
            source_turn_id: turn.turn_id.clone(),
            source_generation_token: turn.generation_token.clone(),
            operation: operation.to_string(),
            canonical_path: canonical_path.clone(),
            target_kind: target_kind.to_string(),
            completed_at_ms,
            result_status: "completed".to_string(),
        };
        let receipt_payload = serde_json::to_string(&payload).map_err(|error| error.to_string())?;
        let signature = identity
            .sign_payload(&receipt_payload)
            .map_err(|error| error.message)?;
        let signature_json =
            serde_json::to_string(&signature).map_err(|error| error.to_string())?;
        transaction
            .execute(
                "INSERT INTO verified_filesystem_contexts (
                    workspace_id, session_id, project_id, source_turn_id,
                    source_generation_token, operation, canonical_path, target_kind,
                    receipt_payload, signature_json, completed_at_ms, result_status,
                    encryption_state
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'completed', ?12)
                 ON CONFLICT(session_id, source_turn_id, operation) DO UPDATE SET
                    canonical_path = excluded.canonical_path,
                    target_kind = excluded.target_kind,
                    receipt_payload = excluded.receipt_payload,
                    signature_json = excluded.signature_json,
                    completed_at_ms = excluded.completed_at_ms,
                    result_status = excluded.result_status,
                    encryption_state = excluded.encryption_state",
                params![
                    workspace_id,
                    turn.session_id,
                    project_id,
                    turn.turn_id,
                    turn.generation_token,
                    operation,
                    canonical_path,
                    target_kind,
                    receipt_payload,
                    signature_json,
                    completed_at_ms,
                    get_current_encryption_state(),
                ],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(context_from_payload(payload, &signature))
    }

    pub fn latest_verified_filesystem_context(
        &self,
        session_id: &str,
        target_kind: &str,
        identity: &SovereignIdentity,
    ) -> Result<Option<VerifiedFilesystemContext>, String> {
        Ok(self
            .verified_filesystem_contexts(session_id, target_kind, identity)?
            .into_iter()
            .next())
    }

    pub fn verified_filesystem_contexts(
        &self,
        session_id: &str,
        target_kind: &str,
        identity: &SovereignIdentity,
    ) -> Result<Vec<VerifiedFilesystemContext>, String> {
        if !matches!(target_kind, "file" | "directory") {
            return Ok(Vec::new());
        }
        let connection = self.open_connection().map_err(|error| error.to_string())?;
        let workspace_id =
            workspace_id_for_chat_session(&connection, session_id, &self.workspace_id)
                .map_err(|error| error.to_string())?;
        let current_project_id = connection
            .query_row(
                "SELECT project_id FROM chat_sessions WHERE id = ?1 AND workspace_id = ?2",
                params![session_id, workspace_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .map_err(|error| error.to_string())?;
        let mut statement = connection
            .prepare(
                "SELECT receipt_payload, signature_json, canonical_path
                 FROM verified_filesystem_contexts
                 WHERE workspace_id = ?1 AND session_id = ?2 AND target_kind = ?3
                   AND result_status = 'completed'
                 ORDER BY completed_at_ms DESC, id DESC LIMIT 32",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![workspace_id, session_id, target_kind], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        let mut contexts = Vec::new();
        for row in rows {
            let (receipt_payload, signature_json, stored_path) =
                row.map_err(|error| error.to_string())?;
            let payload: FilesystemReceiptPayload = match serde_json::from_str(&receipt_payload) {
                Ok(payload) => payload,
                Err(_) => continue,
            };
            let signature: SignatureBlock = match serde_json::from_str(&signature_json) {
                Ok(signature) => signature,
                Err(_) => continue,
            };
            if identity
                .verify_payload(&receipt_payload, &signature)
                .is_err()
                || payload.workspace_id != workspace_id
                || payload.session_id != session_id
                || payload.project_id != current_project_id
                || payload.target_kind != target_kind
                || payload.result_status != "completed"
                || payload.canonical_path != stored_path
            {
                continue;
            }
            let path = Path::new(&payload.canonical_path);
            let valid_target = fs::symlink_metadata(path).ok().is_some_and(|metadata| {
                !metadata.file_type().is_symlink()
                    && ((target_kind == "directory" && metadata.is_dir())
                        || (target_kind == "file" && metadata.is_file()))
            });
            if valid_target {
                contexts.push(context_from_payload(payload, &signature));
            }
        }
        Ok(contexts)
    }

    pub fn active_project_source_directories_for_session(
        &self,
        session_id: &str,
    ) -> Result<Vec<String>, String> {
        let connection = self.open_connection().map_err(|error| error.to_string())?;
        let workspace_id =
            workspace_id_for_chat_session(&connection, session_id, &self.workspace_id)
                .map_err(|error| error.to_string())?;
        let mut statement = connection
            .prepare(
                "SELECT DISTINCT sources.canonical_path
                 FROM chat_sessions AS sessions
                 JOIN projects ON projects.project_id = sessions.project_id
                 JOIN project_sources AS sources ON sources.project_id = projects.project_id
                 WHERE sessions.id = ?1 AND sessions.workspace_id = ?2
                   AND projects.archived_at_ms IS NULL
                   AND sources.grant_state = 'active'
                   AND sources.source_kind IN ('local_folder', 'knowledge_directory')
                 ORDER BY sources.updated_at_ms DESC
                 LIMIT 32",
            )
            .map_err(|error| error.to_string())?;
        let directories = statement
            .query_map(params![session_id, workspace_id], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|error| error.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?;
        Ok(directories)
    }

    pub fn resolve_assistant_content_reference(
        &self,
        session_id: &str,
        prompt: &str,
    ) -> rusqlite::Result<Option<AssistantContentReference>> {
        if !contextual_content_reference_requested(prompt) {
            return Ok(None);
        }
        let messages = self.select_chat_messages(session_id)?;
        let prompt_terms = content_reference_terms(prompt);
        let mut candidates = messages
            .into_iter()
            .filter(|message| message.role == "assistant" && !message.content.trim().is_empty())
            .map(|message| {
                let score = content_reference_terms(&message.content)
                    .iter()
                    .filter(|term| prompt_terms.contains(term))
                    .count();
                (score, message)
            })
            .filter(|(score, _)| prompt_terms.is_empty() || *score > 0)
            .collect::<Vec<_>>();
        candidates.sort_by_key(|(score, message)| (*score, message.created_at_ms, message.id));
        let best = candidates.pop();
        if let (Some((best_score, _)), Some((runner_up_score, _))) =
            (best.as_ref(), candidates.last())
        {
            if best_score == runner_up_score {
                return Ok(None);
            }
        }
        Ok(best.map(|(_, message)| AssistantContentReference {
            message_id: message.id,
            content_digest: sha256_hex(message.content.as_bytes()),
            content: message.content,
        }))
    }

    pub fn prepare_contextual_file_action(
        &self,
        session_id: &str,
        prompt: &str,
        starts_new_action: bool,
        identity: &SovereignIdentity,
    ) -> Result<Option<ContextualFileActionPreparation>, String> {
        if starts_new_action {
            let normalized = prompt.to_ascii_lowercase();
            if !(normalized.contains("markdown") || normalized.contains(".md")) {
                return Ok(None);
            }
            let directory =
                match self.latest_verified_filesystem_context(session_id, "directory", identity)? {
                    Some(directory) => directory,
                    None => return Ok(None),
                };
            let content = match self
                .resolve_assistant_content_reference(session_id, prompt)
                .map_err(|error| error.to_string())?
            {
                Some(content) => content,
                None => return Ok(None),
            };
            self.store_pending_contextual_file_action(session_id, &directory, &content, "md")?;
        }
        let Some(pending) = self.valid_pending_contextual_file_action(session_id, identity)? else {
            return Ok(None);
        };
        let filename = contextual_filename_from_prompt(prompt, &pending.requested_format)
            .unwrap_or_else(|| agent_owned_contextual_filename(prompt, &pending.requested_format));
        let filename = collision_safe_contextual_filename(
            Path::new(&pending.directory_path),
            &filename,
            contextual_filename_from_prompt(prompt, &pending.requested_format).is_none(),
        )?;
        let destination = Path::new(&pending.directory_path).join(&filename);
        let content = self
            .select_chat_messages(session_id)
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|message| message.id == pending.content_message_id && message.role == "assistant")
            .ok_or_else(|| "The selected assistant content is no longer available.".to_string())?;
        if sha256_hex(content.content.as_bytes()) != pending.content_digest {
            return Err(
                "The selected assistant content no longer matches its receipt.".to_string(),
            );
        }
        Ok(Some(ContextualFileActionPreparation::Ready(
            PreparedContextualFileAction {
                directory_path: pending.directory_path,
                destination_path: destination.to_string_lossy().to_string(),
                filename,
                requested_format: pending.requested_format,
                content_message_id: pending.content_message_id,
                content_digest: pending.content_digest,
                content: content.content,
            },
        )))
    }

    pub fn pending_contextual_filename_matches(
        &self,
        session_id: &str,
        prompt: &str,
        identity: &SovereignIdentity,
    ) -> Result<bool, String> {
        let Some(pending) = self.valid_pending_contextual_file_action(session_id, identity)? else {
            return Ok(false);
        };
        Ok(contextual_filename_from_prompt(prompt, &pending.requested_format).is_some())
    }

    fn store_pending_contextual_file_action(
        &self,
        session_id: &str,
        directory: &VerifiedFilesystemContext,
        content: &AssistantContentReference,
        requested_format: &str,
    ) -> Result<(), String> {
        let _guard = self.lock_writes();
        let connection = self.open_connection().map_err(|error| error.to_string())?;
        let workspace_id =
            workspace_id_for_chat_session(&connection, session_id, &self.workspace_id)
                .map_err(|error| error.to_string())?;
        connection
            .execute(
                "INSERT INTO pending_contextual_file_actions (
                    session_id, workspace_id, source_turn_id, directory_path,
                    directory_receipt_digest, content_message_id, content_digest,
                    requested_format, status, updated_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'awaiting_filename', ?9)
                 ON CONFLICT(session_id) DO UPDATE SET
                    workspace_id = excluded.workspace_id,
                    source_turn_id = excluded.source_turn_id,
                    directory_path = excluded.directory_path,
                    directory_receipt_digest = excluded.directory_receipt_digest,
                    content_message_id = excluded.content_message_id,
                    content_digest = excluded.content_digest,
                    requested_format = excluded.requested_format,
                    status = excluded.status,
                    updated_at_ms = excluded.updated_at_ms",
                params![
                    session_id,
                    workspace_id,
                    directory.source_turn_id,
                    directory.canonical_path,
                    directory.verified_receipt_digest,
                    content.message_id,
                    content.content_digest,
                    requested_format,
                    unix_time_ms(),
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn valid_pending_contextual_file_action(
        &self,
        session_id: &str,
        identity: &SovereignIdentity,
    ) -> Result<Option<PendingContextualFileAction>, String> {
        let Some(directory) =
            self.latest_verified_filesystem_context(session_id, "directory", identity)?
        else {
            return Ok(None);
        };
        let connection = self.open_connection().map_err(|error| error.to_string())?;
        let workspace_id =
            workspace_id_for_chat_session(&connection, session_id, &self.workspace_id)
                .map_err(|error| error.to_string())?;
        let pending = connection
            .query_row(
                "SELECT directory_path, directory_receipt_digest, content_message_id,
                        content_digest, requested_format
                 FROM pending_contextual_file_actions
                 WHERE session_id = ?1 AND workspace_id = ?2 AND status = 'awaiting_filename'",
                params![session_id, workspace_id],
                |row| {
                    Ok(PendingContextualFileAction {
                        directory_path: row.get(0)?,
                        directory_receipt_digest: row.get(1)?,
                        content_message_id: row.get(2)?,
                        content_digest: row.get(3)?,
                        requested_format: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(|error| error.to_string())?;
        Ok(pending.filter(|pending| {
            pending.directory_path == directory.canonical_path
                && pending.directory_receipt_digest == directory.verified_receipt_digest
        }))
    }
}

fn agent_owned_contextual_filename(prompt: &str, requested_format: &str) -> String {
    let normalized = prompt.to_ascii_lowercase();
    let stem = if normalized.contains("diagnostic") || normalized.contains("test") {
        "oomu-test-diagnostic-report"
    } else if normalized.contains("search") || normalized.contains("research") {
        "oomu-search-report"
    } else if normalized.contains("contract") || normalized.contains("verification") {
        "oomu-contract-verification-note"
    } else {
        "oomu-conversation-note"
    };
    format!("{stem}.{}", requested_format.trim_start_matches('.'))
}

fn collision_safe_contextual_filename(
    directory: &Path,
    requested: &str,
    agent_owned: bool,
) -> Result<String, String> {
    let candidate = directory.join(requested);
    if !candidate.exists() {
        return Ok(requested.to_string());
    }
    if !agent_owned {
        return Err(format!(
            "{requested} already exists. What different name should I use for the Markdown file?"
        ));
    }
    let requested_path = Path::new(requested);
    let stem = requested_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("oomu-note");
    let extension = requested_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("md");
    for suffix in 2..=10_000usize {
        let filename = format!("{stem}-{suffix}.{extension}");
        if !directory.join(&filename).exists() {
            return Ok(filename);
        }
    }
    Err("OOMU could not find an unused filename in that folder.".to_string())
}

fn contextual_filename_from_prompt(prompt: &str, format: &str) -> Option<String> {
    let extension = format!(".{format}");
    let mut matches = prompt
        .split(|character: char| {
            character.is_whitespace()
                || matches!(
                    character,
                    '`' | '"' | '\'' | '<' | '>' | '(' | ')' | ',' | ';'
                )
        })
        .map(|candidate| {
            candidate.trim_matches(|character: char| matches!(character, '.' | ':' | '!' | '?'))
        })
        .filter(|candidate| candidate.to_ascii_lowercase().ends_with(&extension))
        .filter(|candidate| {
            !candidate.is_empty()
                && candidate.len() <= 128
                && Path::new(candidate).components().count() == 1
        })
        .map(str::to_string)
        .collect::<Vec<_>>();
    matches.sort();
    matches.dedup();
    (matches.len() == 1).then(|| matches.remove(0))
}

fn context_from_payload(
    payload: FilesystemReceiptPayload,
    signature: &SignatureBlock,
) -> VerifiedFilesystemContext {
    VerifiedFilesystemContext {
        session_id: payload.session_id,
        source_turn_id: payload.source_turn_id,
        operation: payload.operation,
        canonical_path: payload.canonical_path,
        target_kind: payload.target_kind,
        verified_receipt_digest: signature.payload_hash.clone(),
        completed_at_ms: payload.completed_at_ms,
        result_status: payload.result_status,
    }
}

fn contextual_content_reference_requested(prompt: &str) -> bool {
    let normalized = prompt.to_ascii_lowercase();
    [
        "your idea",
        "your proposal",
        "write it",
        "save it",
        "put it",
        "take it",
    ]
    .iter()
    .any(|phrase| normalized.contains(phrase))
}

fn content_reference_terms(text: &str) -> Vec<String> {
    const STOP: &[&str] = &[
        "about", "and", "folder", "idea", "into", "it", "save", "take", "that", "the", "this",
        "write", "your",
    ];
    let mut terms = text
        .split(|character: char| !character.is_alphanumeric())
        .map(str::to_ascii_lowercase)
        .filter(|term| term.len() >= 4 && !STOP.contains(&term.as_str()))
        .collect::<Vec<_>>();
    terms.sort();
    terms.dedup();
    terms
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_and_turn(
        engine: &PersistenceEngine,
        suffix: &str,
    ) -> (ChatSessionRecord, ChatTurnPersistenceContext) {
        let session = engine
            .ensure_chat_session(CreateChatSessionRequest {
                agent_id: format!("agent-{suffix}"),
                provider_id: "local_model".to_string(),
                model_id: "gemma-test".to_string(),
                title: Some(format!("Context {suffix}")),
                dynamic_routing_override: None,
                workspace_id: None,
            })
            .unwrap();
        let turn_id = format!("turn-{suffix}");
        let turn = ChatTurnPersistenceContext {
            turn_id: turn_id.clone(),
            generation_token: format!("generation-{suffix}"),
            session_id: session.id.clone(),
            agent_id: session.agent_id.clone(),
            provider_id: session.provider_id.clone(),
            model_id: session.model_id.clone(),
            parent_turn_id: None,
            root_turn_id: turn_id,
            turn_kind: "root".to_string(),
        };
        engine.begin_chat_turn(&turn).unwrap();
        (session, turn)
    }

    #[test]
    fn signed_directory_context_is_same_session_latest_and_tamper_evident() {
        let root = std::env::temp_dir().join(format!("oomu-fs-context-{}", unix_time_ms()));
        let first = root.join("first");
        let second = root.join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let identity = SovereignIdentity::initialize_ephemeral();
        let (session, first_turn) = session_and_turn(&engine, "first");
        let (other_session, _) = session_and_turn(&engine, "other");

        let recorded = engine
            .record_verified_filesystem_context(
                &first_turn,
                "file_list",
                first.to_str().unwrap(),
                "directory",
                &identity,
            )
            .unwrap();
        assert_eq!(
            recorded.canonical_path,
            first.canonicalize().unwrap().to_string_lossy()
        );
        assert!(engine
            .latest_verified_filesystem_context(&other_session.id, "directory", &identity)
            .unwrap()
            .is_none());

        let second_turn = ChatTurnPersistenceContext {
            turn_id: "turn-second".to_string(),
            generation_token: "generation-second".to_string(),
            root_turn_id: "turn-second".to_string(),
            ..first_turn.clone()
        };
        engine.begin_chat_turn(&second_turn).unwrap();
        engine
            .record_verified_filesystem_context(
                &second_turn,
                "file_list",
                second.to_str().unwrap(),
                "directory",
                &identity,
            )
            .unwrap();
        let latest = engine
            .latest_verified_filesystem_context(&session.id, "directory", &identity)
            .unwrap()
            .unwrap();
        assert_eq!(
            latest.canonical_path,
            second.canonicalize().unwrap().to_string_lossy()
        );

        engine
            .open_connection()
            .unwrap()
            .execute(
                "UPDATE verified_filesystem_contexts SET canonical_path = '/tmp/forged' WHERE session_id = ?1",
                params![session.id],
            )
            .unwrap();
        std::fs::remove_dir_all(&second).unwrap();
        assert!(engine
            .latest_verified_filesystem_context(&session.id, "directory", &identity)
            .unwrap()
            .is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn assistant_content_reference_is_bound_to_one_message_and_digest() {
        let root = std::env::temp_dir().join(format!("oomu-content-context-{}", unix_time_ms()));
        std::fs::create_dir_all(&root).unwrap();
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let (session, _) = session_and_turn(&engine, "content");
        engine
            .insert_chat_message(
                &session.id,
                &session.agent_id,
                "assistant",
                "Use a contract verification layer with signed boundary receipts.",
            )
            .unwrap();
        let reference = engine
            .resolve_assistant_content_reference(
                &session.id,
                "Take your idea about the contract verification layer and write it into that folder.",
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            reference.content_digest,
            sha256_hex(reference.content.as_bytes())
        );
        assert!(reference.content.contains("signed boundary receipts"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn contextual_file_target_retains_folder_content_and_format_through_filename_only_follow_up() {
        let root = std::env::temp_dir().join(format!("oomu-contextual-file-{}", unix_time_ms()));
        let folder = root.join("target");
        std::fs::create_dir_all(&folder).unwrap();
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let identity = SovereignIdentity::initialize_ephemeral();
        let (session, turn) = session_and_turn(&engine, "pending-file");
        engine
            .record_verified_filesystem_context(
                &turn,
                "file_list",
                folder.to_str().unwrap(),
                "directory",
                &identity,
            )
            .unwrap();
        let proposal = "Use a contract verification layer.\n\n- [ ] Bind every boundary receipt.";
        engine
            .insert_chat_message(&session.id, &session.agent_id, "assistant", proposal)
            .unwrap();
        let prompt = "Take your idea about the contract verification layer and write it into that folder. Use markdown format.";
        let Some(ContextualFileActionPreparation::Ready(prepared)) = engine
            .prepare_contextual_file_action(&session.id, prompt, true, &identity)
            .unwrap()
        else {
            panic!("agent-owned filename should complete the grounded action");
        };
        assert_eq!(prepared.content, proposal);
        assert_eq!(prepared.requested_format, "md");
        assert_eq!(prepared.filename, "oomu-contract-verification-note.md");
        assert_eq!(
            prepared.destination_path,
            folder
                .canonicalize()
                .unwrap()
                .join("oomu-contract-verification-note.md")
                .to_string_lossy()
        );
        assert_eq!(prepared.content_digest, sha256_hex(proposal.as_bytes()));
        std::fs::remove_dir_all(root).unwrap();
    }
}
