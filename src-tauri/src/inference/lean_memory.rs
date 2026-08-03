use super::{truncate_for_prompt, AgentIdentityContext, AgentMemoryEntry, ContextBlock};

pub(super) fn build_lean_chat_long_term_blocks(
    identity_context: &AgentIdentityContext,
    mod_knowledge_context: Option<&str>,
) -> Vec<ContextBlock> {
    let mut blocks = Vec::with_capacity(2);
    if !identity_context.memories.is_empty() {
        blocks.push(ContextBlock::new(
            "Dynamic Durable Memory Matches",
            format_agent_memory_matches(&identity_context.memories),
        ));
    }
    if let Some(context) = mod_knowledge_context
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        blocks.push(ContextBlock::new(
            "Isolated Mod Knowledge Retrieval",
            context,
        ));
    }
    blocks
}

pub(super) fn format_agent_memory_matches(memories: &[AgentMemoryEntry]) -> String {
    if memories.is_empty() {
        return "Source: signed SQLite agent_memory_entries keyword retrieval. Limit: top 3.\n- No durable memories matched this turn yet.".to_string();
    }
    let lines = memories
        .iter()
        .take(3)
        .map(|memory| {
            format!(
                "- [{} / {} / confidence {:.2}] {}",
                memory.memory_kind,
                memory.scope,
                memory.confidence,
                truncate_for_prompt(&memory.content, 900)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let freshness = memories
        .iter()
        .any(|memory| memory.memory_kind == "daily_journal")
        .then_some("\nImported journal matches are refreshed for this turn and ordered for the user's request. When they conflict with an earlier chat answer, use the first imported journal match and cite its date and source filename.")
        .unwrap_or_default();
    format!("Source: signed SQLite agent_memory_entries keyword retrieval. Limit: top 3.{freshness}\n{lines}")
}
