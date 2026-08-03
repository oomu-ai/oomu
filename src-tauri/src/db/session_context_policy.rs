use super::*;

pub(super) fn build_extractive_checkpoint(messages: &[&ChatMessageRecord]) -> String {
    let source_chars = messages
        .iter()
        .map(|message| message.content.chars().count())
        .sum::<usize>();
    let mut roles = Vec::new();
    for message in messages {
        if roles.iter().any(|(role, _)| role == &message.role) {
            continue;
        }
        let digest = sha256_hex(message.content.as_bytes());
        roles.push((message.role.clone(), digest[..12].to_string()));
    }
    let evidence = roles
        .iter()
        .map(|(role, digest)| format!("role={role} sha256={digest}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut checkpoint = format!(
        "Compacted source evidence; no goals, conclusions, or pending tasks were inferred.\n{evidence}"
    );
    let remaining = source_chars.saturating_sub(checkpoint.chars().count() + 1);
    if let Some(message) = messages.iter().max_by_key(|message| message.content.len()) {
        let excerpt = checkpoint_sentence(&message.content, remaining.saturating_sub(8).min(160));
        if !excerpt.is_empty() {
            checkpoint.push_str(&format!("\nextract={excerpt}"));
        }
    }
    checkpoint
        .chars()
        .take(source_chars.saturating_sub(1))
        .collect()
}

fn checkpoint_sentence(content: &str, max_chars: usize) -> String {
    let normalized = content.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= max_chars {
        return normalized;
    }
    if max_chars <= 3 {
        return normalized.chars().take(max_chars).collect();
    }
    format!(
        "{}...",
        normalized.chars().take(max_chars - 3).collect::<String>()
    )
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextHorizonStatus {
    pub estimated_tokens_used: usize,
    pub tokens_total: usize,
    pub working_budget_tokens: usize,
    pub provider_max_tokens: usize,
    pub estimated_percentage_used: f32,
    pub active_model_id: String,
    pub is_cloud_model: bool,
    pub auto_compaction_threshold_percent: u8,
    pub auto_compaction_enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_compaction: Option<ContextCompactionResult>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveSessionContextPolicyRequest {
    pub session_id: String,
    pub auto_compaction_threshold_percent: u8,
    pub auto_compaction_enabled: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionContextPolicyRecord {
    pub auto_compaction_threshold_percent: u8,
    pub auto_compaction_enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompactChatSessionRequest {
    pub session_id: String,
    pub target_percent: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextCompactionResult {
    pub session_id: String,
    pub before_tokens: usize,
    pub after_tokens: usize,
    pub target_tokens: usize,
    pub compacted_message_count: usize,
    pub preserved_message_count: usize,
    pub next_request_tokens: usize,
    pub threshold_percent: u8,
}

impl PersistenceEngine {
    pub fn session_context_status(
        &self,
        session_id: &str,
    ) -> rusqlite::Result<ContextHorizonStatus> {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return Err(rusqlite::Error::InvalidParameterName(
                "session_id must not be empty".to_string(),
            ));
        }
        let connection = self.open_connection()?;
        let workspace_id =
            workspace_id_for_chat_session(&connection, session_id, &self.workspace_id)?;
        let policy = policy_for_connection(&connection, session_id)?;
        let config = select_session_config_for_connection(&connection, session_id)?;
        let provider_id = config
            .as_ref()
            .and_then(|record| record.local_provider_type.as_deref())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("")
            .to_string();
        let model_id = config
            .as_ref()
            .and_then(|record| record.model_id.as_deref())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("local-context")
            .to_string();
        let configured_budget = config
            .as_ref()
            .map(|record| record.context_budget.max(1) as usize)
            .unwrap_or(DEFAULT_LOCAL_CONTEXT_TOKENS);
        let is_cloud_model = !route_uses_local_context(&provider_id, &model_id);
        let provider_max_tokens = resolved_context_horizon_tokens(
            &provider_id,
            &model_id,
            configured_budget,
            is_cloud_model,
        );
        let total_chars = connection.query_row(
            "SELECT COALESCE(SUM(LENGTH(content)), 0) FROM chat_messages
             WHERE workspace_id = ?1 AND session_id = ?2
               AND COALESCE(is_compacted, 0) = 0
               AND COALESCE(json_extract(metadata_json, '$.uiOnlyCheckpoint'), 0) = 0",
            params![workspace_id, session_id],
            |row| row.get::<_, i64>(0),
        )? as usize;
        let estimated_tokens_used = (total_chars + 3) / 4;
        let estimated_percentage_used =
            ((estimated_tokens_used as f32) / (configured_budget as f32)).min(1.0);
        let last_compaction = select_last_compaction(&connection, &workspace_id, session_id)?;
        Ok(ContextHorizonStatus {
            estimated_tokens_used,
            tokens_total: provider_max_tokens,
            working_budget_tokens: configured_budget,
            provider_max_tokens,
            estimated_percentage_used,
            active_model_id: model_id,
            is_cloud_model,
            auto_compaction_threshold_percent: policy.auto_compaction_threshold_percent,
            auto_compaction_enabled: policy.auto_compaction_enabled,
            last_compaction,
        })
    }

    pub fn session_context_policy(
        &self,
        session_id: &str,
    ) -> rusqlite::Result<SessionContextPolicyRecord> {
        let connection = self.open_connection()?;
        policy_for_connection(&connection, session_id.trim())
    }

    pub fn save_session_context_policy(
        &self,
        request: &SaveSessionContextPolicyRequest,
    ) -> rusqlite::Result<SessionContextPolicyRecord> {
        let session_id = request.session_id.trim();
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        connection.execute(
            "INSERT INTO active_session_configs (
                 session_id, reasoning_depth, context_budget,
                 auto_compaction_enabled, auto_compaction_threshold_percent, updated_at
             ) VALUES (?1, 'medium', ?2, ?3, ?4, CURRENT_TIMESTAMP)
             ON CONFLICT(session_id) DO UPDATE SET
                 auto_compaction_enabled = excluded.auto_compaction_enabled,
                 auto_compaction_threshold_percent = excluded.auto_compaction_threshold_percent,
                 updated_at = CURRENT_TIMESTAMP",
            params![
                session_id,
                settings::DEFAULT_CONTEXT_BUDGET as i64,
                i64::from(request.auto_compaction_enabled),
                i64::from(request.auto_compaction_threshold_percent),
            ],
        )?;
        policy_for_connection(&connection, session_id)
    }

    pub fn compact_chat_session(
        &self,
        request: &CompactChatSessionRequest,
    ) -> rusqlite::Result<ContextCompactionResult> {
        let session_id = request.session_id.trim();
        let policy = self.session_context_policy(session_id)?;
        let threshold_percent = request
            .target_percent
            .unwrap_or(policy.auto_compaction_threshold_percent);
        let before = self.session_context_status(session_id)?;
        let response = self.compact_session_messages(session_id)?;
        let after = self.session_context_status(session_id)?;
        let active_messages = self.select_chat_messages(session_id)?;
        let result = ContextCompactionResult {
            session_id: session_id.to_string(),
            before_tokens: before.estimated_tokens_used,
            after_tokens: after.estimated_tokens_used,
            target_tokens: before
                .working_budget_tokens
                .saturating_mul(usize::from(threshold_percent))
                / 100,
            compacted_message_count: response.compacted_messages,
            preserved_message_count: active_messages
                .iter()
                .filter(|message| message.compaction_type.as_deref() != Some("summary_anchor"))
                .count(),
            next_request_tokens: after.estimated_tokens_used,
            threshold_percent,
        };
        if let Some(anchor_message_id) = response.anchor_message_id {
            self.record_context_compaction_receipt(anchor_message_id, &result)?;
        }
        Ok(result)
    }

    fn record_context_compaction_receipt(
        &self,
        anchor_message_id: i64,
        result: &ContextCompactionResult,
    ) -> rusqlite::Result<()> {
        let receipt = serde_json::to_string(result).map_err(json_to_sql_error)?;
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        connection.execute(
            "UPDATE chat_messages SET metadata_json = json_set(
                 COALESCE(metadata_json, '{}'), '$.compactionReceipt', json(?1)
             ) WHERE id = ?2 AND compaction_type = 'summary_anchor'",
            params![receipt, anchor_message_id],
        )?;
        Ok(())
    }
}

fn policy_for_connection(
    connection: &Connection,
    session_id: &str,
) -> rusqlite::Result<SessionContextPolicyRecord> {
    connection
        .query_row(
            "SELECT auto_compaction_enabled, auto_compaction_threshold_percent
             FROM active_session_configs WHERE session_id = ?1",
            params![session_id],
            |row| {
                Ok(SessionContextPolicyRecord {
                    auto_compaction_enabled: row.get::<_, i64>(0)? != 0,
                    auto_compaction_threshold_percent: u8::try_from(row.get::<_, i64>(1)?)
                        .unwrap_or(settings::DEFAULT_AUTO_COMPACTION_THRESHOLD_PERCENT),
                })
            },
        )
        .optional()
        .map(|policy| {
            policy.unwrap_or(SessionContextPolicyRecord {
                auto_compaction_enabled: true,
                auto_compaction_threshold_percent:
                    settings::DEFAULT_AUTO_COMPACTION_THRESHOLD_PERCENT,
            })
        })
}

fn select_last_compaction(
    connection: &Connection,
    workspace_id: &str,
    session_id: &str,
) -> rusqlite::Result<Option<ContextCompactionResult>> {
    let receipt = connection
        .query_row(
            "SELECT json_extract(metadata_json, '$.compactionReceipt') FROM chat_messages
         WHERE workspace_id = ?1 AND session_id = ?2 AND compaction_type = 'summary_anchor'
           AND json_type(metadata_json, '$.compactionReceipt') = 'object'
         ORDER BY id DESC LIMIT 1",
            params![workspace_id, session_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    receipt
        .map(|value| serde_json::from_str(&value).map_err(json_from_sql_error))
        .transpose()
}
