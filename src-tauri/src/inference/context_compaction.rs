use super::*;

pub(super) fn maybe_compact_standard_chat_history(
    persistence: &PersistenceEngine,
    session_id: &str,
    token_budget: usize,
    grounding_bypass_active: bool,
    pending_content: Option<&str>,
) -> rusqlite::Result<bool> {
    if grounding_bypass_active {
        return Ok(false);
    }

    let policy = persistence.session_context_policy(session_id)?;
    if !policy.auto_compaction_enabled {
        return Ok(false);
    }
    let active_messages = persistence.select_chat_messages(session_id)?;
    let estimated_tokens = estimate_active_chat_history_tokens(&active_messages, pending_content);
    let threshold_tokens =
        token_budget.saturating_mul(usize::from(policy.auto_compaction_threshold_percent)) / 100;
    if estimated_tokens <= threshold_tokens {
        return Ok(false);
    }

    let response = persistence.compact_chat_session(&crate::db::CompactChatSessionRequest {
        session_id: session_id.to_string(),
        target_percent: Some(policy.auto_compaction_threshold_percent),
    })?;
    if response.compacted_message_count == 0 {
        return Ok(false);
    }

    eprintln!(
        "CHAT_HISTORY_AUTO_COMPACTION session_id={} estimated_tokens={} budget_tokens={} threshold_percent={} threshold_tokens={} compacted_messages={} after_tokens={}",
        session_id,
        estimated_tokens,
        token_budget,
        policy.auto_compaction_threshold_percent,
        threshold_tokens,
        response.compacted_message_count,
        response.after_tokens,
    );
    Ok(true)
}

fn estimate_active_chat_history_tokens(
    messages: &[crate::db::ChatMessageRecord],
    pending_content: Option<&str>,
) -> usize {
    let message_tokens = messages.iter().fold(0usize, |total, message| {
        if message
            .metadata_json
            .as_deref()
            .and_then(|metadata| serde_json::from_str::<Value>(metadata).ok())
            .and_then(|metadata| metadata.get("uiOnlyCheckpoint").and_then(Value::as_bool))
            .unwrap_or(false)
        {
            return total;
        }
        total
            .saturating_add(4)
            .saturating_add(estimate_text_tokens(&message.role))
            .saturating_add(estimate_text_tokens(&message.content))
    });

    pending_content
        .map(estimate_text_tokens)
        .unwrap_or_default()
        .saturating_add(message_tokens)
}
