use crate::inference::{ChatAttachment, InferenceMessage};

const TIER1_WEIGHT: usize = 45;
const TIER2_WEIGHT: usize = 40;
const TIER3_WEIGHT: usize = 15;
const WEIGHT_TOTAL: usize = TIER1_WEIGHT + TIER2_WEIGHT + TIER3_WEIGHT;
#[cfg(test)]
const DEFAULT_WORKING_TURN_LIMIT: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContextBlock {
    pub label: String,
    pub content: String,
}

impl ContextBlock {
    pub(crate) fn new(label: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            content: content.into(),
        }
    }

    fn formatted(&self) -> String {
        let label = self.label.trim();
        let content = self.content.trim();
        if label.is_empty() {
            content.to_string()
        } else {
            format!("{label}\n{content}")
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ContextAssemblyRequest {
    pub static_core_blocks: Vec<ContextBlock>,
    pub working_context_blocks: Vec<ContextBlock>,
    pub working_messages: Vec<InferenceMessage>,
    pub long_term_blocks: Vec<ContextBlock>,
    pub token_budget: Option<usize>,
    pub working_turn_limit: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct ContextAssembly {
    pub system_prompt: String,
    pub messages: Vec<InferenceMessage>,
    pub estimated_tokens: usize,
    pub core_tokens: usize,
    pub working_context_tokens: usize,
    pub working_tokens: usize,
    pub long_term_tokens: usize,
    pub dropped_working_context_blocks: usize,
    pub dropped_working_messages: usize,
    pub dropped_long_term_blocks: usize,
    pub latest_user_message_condensed: bool,
    pub latest_user_attachments_preserved: bool,
}

impl ContextAssembly {
    pub(crate) fn condensation(&self, budget_tokens: usize) -> Option<(usize, bool)> {
        self.latest_user_message_condensed
            .then_some((budget_tokens, self.latest_user_attachments_preserved))
    }
}

pub(crate) fn observe_context_assembly(
    assembly: &ContextAssembly,
    agent_id: &str,
    session_id: &str,
    budget_tokens: Option<usize>,
) {
    eprintln!(
        "OOMU_CONTEXT_ENGINE agent_id={} session_id={} budget_tokens={} estimated_tokens={} core_tokens={} working_context_tokens={} working_tokens={} long_term_tokens={} dropped_working_context_blocks={} dropped_working_messages={} dropped_long_term_blocks={}",
        agent_id,
        session_id,
        budget_tokens
            .map(|tokens| tokens.to_string())
            .unwrap_or_else(|| "provider-defined".to_string()),
        assembly.estimated_tokens,
        assembly.core_tokens,
        assembly.working_context_tokens,
        assembly.working_tokens,
        assembly.long_term_tokens,
        assembly.dropped_working_context_blocks,
        assembly.dropped_working_messages,
        assembly.dropped_long_term_blocks
    );
}

pub(crate) fn assemble_context(request: ContextAssemblyRequest) -> ContextAssembly {
    let token_budget = request.token_budget;
    let working_turn_limit = request.working_turn_limit.max(1);
    let core_prompt = format_blocks(&request.static_core_blocks);
    let core_tokens = estimate_text_tokens(&core_prompt);
    let working_quota = token_budget
        .map(|budget| weighted_quota(budget, TIER2_WEIGHT))
        .unwrap_or(usize::MAX);
    let long_term_quota = token_budget
        .map(|budget| weighted_quota(budget, TIER3_WEIGHT))
        .unwrap_or(usize::MAX);
    let lower_tier_budget = token_budget
        .map(|budget| budget.saturating_sub(core_tokens))
        .unwrap_or(usize::MAX);
    let effective_working_quota = if token_budget.is_some() {
        working_quota.min(lower_tier_budget)
    } else {
        working_quota
    };

    let working_context_quota = if token_budget.is_some() {
        effective_working_quota / 3
    } else {
        usize::MAX
    };
    let working_context = select_context_blocks(
        &request.working_context_blocks,
        working_context_quota,
        token_budget.is_some(),
    );
    let message_quota = if token_budget.is_some() {
        effective_working_quota.saturating_sub(working_context.tokens)
    } else {
        usize::MAX
    };
    let working = select_working_messages(
        &request.working_messages,
        message_quota,
        working_turn_limit,
        token_budget.is_some(),
    );
    let remaining_after_working = lower_tier_budget
        .saturating_sub(working_context.tokens)
        .saturating_sub(working.tokens);
    let effective_long_term_quota = if token_budget.is_some() {
        long_term_quota.min(remaining_after_working)
    } else {
        long_term_quota
    };
    let long_term = select_context_blocks(
        &request.long_term_blocks,
        effective_long_term_quota,
        token_budget.is_some(),
    );

    let system_prompt =
        join_system_prompt(&core_prompt, &working_context.content, &long_term.content);
    let estimated_tokens = core_tokens + working_context.tokens + working.tokens + long_term.tokens;

    ContextAssembly {
        system_prompt,
        messages: working.messages,
        estimated_tokens,
        core_tokens,
        working_context_tokens: working_context.tokens,
        working_tokens: working.tokens,
        long_term_tokens: long_term.tokens,
        dropped_working_context_blocks: request
            .working_context_blocks
            .len()
            .saturating_sub(working_context.selected_count),
        dropped_working_messages: request
            .working_messages
            .len()
            .saturating_sub(working.selected_original_count),
        dropped_long_term_blocks: request
            .long_term_blocks
            .len()
            .saturating_sub(long_term.selected_count),
        latest_user_message_condensed: working.latest_message_condensed,
        latest_user_attachments_preserved: working.latest_attachments_preserved,
    }
}

#[derive(Debug, Clone)]
struct WorkingSelection {
    messages: Vec<InferenceMessage>,
    tokens: usize,
    selected_original_count: usize,
    latest_message_condensed: bool,
    latest_attachments_preserved: bool,
}

fn select_working_messages(
    messages: &[InferenceMessage],
    quota: usize,
    turn_limit: usize,
    enforce_quota: bool,
) -> WorkingSelection {
    if messages.is_empty() {
        return WorkingSelection {
            messages: Vec::new(),
            tokens: 0,
            selected_original_count: 0,
            latest_message_condensed: false,
            latest_attachments_preserved: true,
        };
    }

    let latest_user_index = messages
        .iter()
        .rposition(|message| message.role.eq_ignore_ascii_case("user"))
        .unwrap_or_else(|| messages.len().saturating_sub(1));
    let start = working_window_start(messages, latest_user_index, turn_limit);
    let mut window = messages[start..=latest_user_index].to_vec();
    let original_count = window.len();

    if !enforce_quota {
        let tokens = window.iter().map(estimate_message_tokens).sum();
        return WorkingSelection {
            messages: window,
            tokens,
            selected_original_count: original_count,
            latest_message_condensed: false,
            latest_attachments_preserved: true,
        };
    }

    let mut selected_rev = Vec::<InferenceMessage>::new();
    let mut used = 0usize;
    let mut latest_message_condensed = false;
    let mut latest_attachments_preserved = true;
    for (offset, message) in window.iter().enumerate().rev() {
        let message_tokens = estimate_message_tokens(message);
        let is_latest = offset == window.len().saturating_sub(1);
        if is_latest || used + message_tokens <= quota {
            let mut next = message.clone();
            if is_latest && message_tokens > quota {
                next = truncate_message_to_budget(next, quota);
                latest_message_condensed = message_was_condensed(message, &next);
                latest_attachments_preserved = message
                    .attachments
                    .iter()
                    .zip(&next.attachments)
                    .all(|(before, after)| {
                        before.text.as_deref().is_none_or(str::is_empty)
                            || after
                                .text
                                .as_deref()
                                .is_some_and(|text| !text.trim().is_empty())
                    });
                if !message_has_dispatchable_content(&next)
                    && message_has_dispatchable_content(message)
                {
                    next = message.clone();
                    latest_message_condensed = false;
                    latest_attachments_preserved = true;
                }
            }
            used += estimate_message_tokens(&next);
            selected_rev.push(next);
        }
    }

    selected_rev.reverse();
    window = selected_rev;
    let tokens = window.iter().map(estimate_message_tokens).sum();
    let selected_count = window.len();
    WorkingSelection {
        messages: window,
        tokens,
        selected_original_count: selected_count,
        latest_message_condensed,
        latest_attachments_preserved,
    }
}

fn working_window_start(
    messages: &[InferenceMessage],
    latest_user_index: usize,
    turn_limit: usize,
) -> usize {
    let mut seen_user_turns = 0usize;
    for index in (0..=latest_user_index).rev() {
        if messages[index].role.eq_ignore_ascii_case("user") {
            seen_user_turns += 1;
            if seen_user_turns == turn_limit {
                return index;
            }
        }
    }
    0
}

#[derive(Debug, Clone)]
struct ContextBlockSelection {
    content: String,
    tokens: usize,
    selected_count: usize,
}

fn select_context_blocks(
    blocks: &[ContextBlock],
    quota: usize,
    enforce_quota: bool,
) -> ContextBlockSelection {
    if blocks.is_empty() {
        return ContextBlockSelection {
            content: String::new(),
            tokens: 0,
            selected_count: 0,
        };
    }

    let mut selected = Vec::new();
    let mut used = 0usize;
    let mut selected_count = 0usize;
    for block in blocks {
        let formatted = block.formatted();
        let block_tokens = estimate_text_tokens(&formatted);
        if !enforce_quota || used + block_tokens <= quota {
            used += block_tokens;
            selected.push(formatted);
            selected_count += 1;
            continue;
        }

        let remaining = quota.saturating_sub(used);
        let truncated = truncate_text_to_estimated_tokens(&formatted, remaining);
        if !truncated.trim().is_empty() {
            used += estimate_text_tokens(&truncated);
            selected.push(truncated);
            selected_count += 1;
        }
        break;
    }

    ContextBlockSelection {
        content: selected.join("\n\n"),
        tokens: used,
        selected_count,
    }
}

fn format_blocks(blocks: &[ContextBlock]) -> String {
    blocks
        .iter()
        .map(ContextBlock::formatted)
        .filter(|block| !block.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn join_system_prompt(core_prompt: &str, working_prompt: &str, long_term_prompt: &str) -> String {
    let mut sections = Vec::new();
    if !core_prompt.trim().is_empty() {
        sections.push(core_prompt.trim().to_string());
    }
    if !working_prompt.trim().is_empty() {
        sections.push(format!(
            "Tier 2 Dynamic Working Context\n{}",
            working_prompt.trim()
        ));
    }
    if !long_term_prompt.trim().is_empty() {
        sections.push(format!(
            "Tier 3 Dynamic Long-Term Context\n{}",
            long_term_prompt.trim()
        ));
    }
    sections.join("\n\n")
}

fn weighted_quota(total_budget: usize, weight: usize) -> usize {
    total_budget.saturating_mul(weight) / WEIGHT_TOTAL
}

pub(crate) fn estimate_message_tokens(message: &InferenceMessage) -> usize {
    4 + estimate_text_tokens(&message.role)
        + estimate_text_tokens(&message.content)
        + message
            .attachments
            .iter()
            .map(estimate_attachment_tokens)
            .sum::<usize>()
}

fn message_has_dispatchable_content(message: &InferenceMessage) -> bool {
    !message.content.trim().is_empty()
        || message.attachments.iter().any(|attachment| {
            attachment
                .text
                .as_deref()
                .map(str::trim)
                .is_some_and(|value| !value.is_empty())
                || attachment
                    .data_base64
                    .as_deref()
                    .map(str::trim)
                    .is_some_and(|value| !value.is_empty())
        })
}

fn estimate_attachment_tokens(attachment: &ChatAttachment) -> usize {
    estimate_text_tokens(&attachment.name)
        + estimate_text_tokens(&attachment.mime_type)
        + attachment
            .text
            .as_deref()
            .map(estimate_text_tokens)
            .unwrap_or_default()
}

pub(crate) fn estimate_text_tokens(value: &str) -> usize {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return 0;
    }
    let char_estimate = trimmed.chars().count().div_ceil(4);
    let word_estimate = trimmed.split_whitespace().count();
    char_estimate.max(word_estimate).max(1)
}

fn truncate_message_to_budget(
    mut message: InferenceMessage,
    budget_tokens: usize,
) -> InferenceMessage {
    let fixed_message_tokens = 4 + estimate_text_tokens(&message.role);
    let attachment_metadata_tokens = message
        .attachments
        .iter()
        .map(|attachment| {
            estimate_text_tokens(&attachment.name) + estimate_text_tokens(&attachment.mime_type)
        })
        .sum::<usize>();
    let text_budget = budget_tokens
        .saturating_sub(fixed_message_tokens)
        .saturating_sub(attachment_metadata_tokens);
    let text_needs = std::iter::once(estimate_text_tokens(&message.content))
        .chain(message.attachments.iter().map(|attachment| {
            attachment
                .text
                .as_deref()
                .map(estimate_text_tokens)
                .unwrap_or_default()
        }))
        .collect::<Vec<_>>();
    let allocations = recency_weighted_token_allocations(&text_needs, text_budget);

    message.content = truncate_text_to_estimated_tokens(
        &message.content,
        allocations.first().copied().unwrap_or_default(),
    );
    for (attachment, allocation) in message
        .attachments
        .iter_mut()
        .zip(allocations.into_iter().skip(1))
    {
        if let Some(text) = attachment.text.as_deref() {
            let condensed = truncate_text_to_estimated_tokens(text, allocation);
            attachment.text = (!condensed.is_empty()).then_some(condensed);
        }
    }
    message
}

fn recency_weighted_token_allocations(needs: &[usize], budget: usize) -> Vec<usize> {
    let mut allocations = vec![0usize; needs.len()];
    let mut unsatisfied = needs
        .iter()
        .enumerate()
        .filter_map(|(index, need)| (*need > 0).then_some(index))
        .collect::<Vec<_>>();
    let mut remaining = budget;

    while remaining > 0 && !unsatisfied.is_empty() {
        let total_weight = unsatisfied
            .iter()
            .map(|index| 100usize.saturating_add(index.saturating_mul(5)))
            .sum::<usize>()
            .max(1);
        let mut spent = 0usize;
        unsatisfied.retain(|index| {
            let outstanding = needs[*index].saturating_sub(allocations[*index]);
            let weight = 100usize.saturating_add(index.saturating_mul(5));
            let weighted_share = remaining
                .saturating_mul(weight)
                .div_ceil(total_weight)
                .max(1);
            let granted = outstanding
                .min(weighted_share)
                .min(remaining.saturating_sub(spent));
            allocations[*index] += granted;
            spent += granted;
            allocations[*index] < needs[*index]
        });
        if spent == 0 {
            break;
        }
        remaining = remaining.saturating_sub(spent);
    }

    allocations
}

fn message_was_condensed(before: &InferenceMessage, after: &InferenceMessage) -> bool {
    before.content.trim() != after.content.trim()
        || before
            .attachments
            .iter()
            .zip(&after.attachments)
            .any(|(left, right)| {
                left.text.as_deref().map(str::trim) != right.text.as_deref().map(str::trim)
            })
}

fn truncate_text_to_estimated_tokens(value: &str, max_tokens: usize) -> String {
    let trimmed = value.trim();
    if estimate_text_tokens(trimmed) <= max_tokens {
        return trimmed.to_string();
    }
    if max_tokens == 0 {
        return String::new();
    }

    let marker = "\n[truncated]";
    let marker_tokens = estimate_text_tokens(marker);
    if max_tokens <= marker_tokens {
        return String::new();
    }

    let mut truncated = trimmed
        .chars()
        .take(max_tokens.saturating_sub(marker_tokens).saturating_mul(4))
        .collect::<String>();
    loop {
        let candidate = format!("{truncated}{marker}");
        if estimate_text_tokens(&candidate) <= max_tokens {
            return candidate;
        }
        if truncated.pop().is_none() {
            return String::new();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(role: &str, content: &str) -> InferenceMessage {
        InferenceMessage {
            role: role.to_string(),
            content: content.to_string(),
            attachments: Vec::new(),
        }
    }

    fn attachment(name: &str, text: &str) -> ChatAttachment {
        ChatAttachment {
            name: name.to_string(),
            mime_type: "application/json".to_string(),
            byte_count: text.len(),
            data_base64: None,
            text: Some(text.to_string()),
            approved_file_receipt: None,
        }
    }

    #[test]
    fn static_core_is_not_truncated_when_over_nominal_tier() {
        let core = "core directive ".repeat(200);
        let assembly = assemble_context(ContextAssemblyRequest {
            static_core_blocks: vec![ContextBlock::new("Core", core.clone())],
            working_context_blocks: Vec::new(),
            working_messages: vec![message("user", "latest request")],
            long_term_blocks: vec![ContextBlock::new("Memory", "old preference")],
            token_budget: Some(64),
            working_turn_limit: DEFAULT_WORKING_TURN_LIMIT,
        });

        assert!(assembly.system_prompt.contains(core.trim()));
        assert_eq!(assembly.messages.len(), 1);
    }

    #[test]
    fn working_memory_keeps_last_five_user_turns() {
        let mut messages = Vec::new();
        for index in 0..7 {
            messages.push(message("user", &format!("user turn {index}")));
            messages.push(message("assistant", &format!("assistant turn {index}")));
        }
        messages.push(message("user", "latest user turn"));

        let assembly = assemble_context(ContextAssemblyRequest {
            static_core_blocks: vec![ContextBlock::new("Core", "system")],
            working_context_blocks: Vec::new(),
            working_messages: messages,
            long_term_blocks: Vec::new(),
            token_budget: None,
            working_turn_limit: DEFAULT_WORKING_TURN_LIMIT,
        });
        let contents = assembly
            .messages
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>();

        assert_eq!(contents.first().copied(), Some("user turn 3"));
        assert_eq!(contents.last().copied(), Some("latest user turn"));
        assert_eq!(
            assembly
                .messages
                .iter()
                .filter(|message| message.role.eq_ignore_ascii_case("user"))
                .count(),
            5
        );
    }

    #[test]
    fn long_term_blocks_are_budgeted_after_working_memory() {
        let assembly = assemble_context(ContextAssemblyRequest {
            static_core_blocks: vec![ContextBlock::new("Core", "system")],
            working_context_blocks: Vec::new(),
            working_messages: vec![message("user", "latest request")],
            long_term_blocks: vec![
                ContextBlock::new("Memory A", "alpha ".repeat(80)),
                ContextBlock::new("Memory B", "beta ".repeat(80)),
            ],
            token_budget: Some(120),
            working_turn_limit: DEFAULT_WORKING_TURN_LIMIT,
        });

        assert!(assembly
            .system_prompt
            .contains("Tier 3 Dynamic Long-Term Context"));
        assert!(assembly.system_prompt.contains("[truncated]"));
        assert!(assembly.dropped_long_term_blocks > 0);
    }

    #[test]
    fn working_context_blocks_are_tier_two_and_budgeted() {
        let assembly = assemble_context(ContextAssemblyRequest {
            static_core_blocks: vec![ContextBlock::new("Core", "system")],
            working_context_blocks: vec![ContextBlock::new("Terminal", "stdout ".repeat(120))],
            working_messages: vec![message("user", "latest request")],
            long_term_blocks: Vec::new(),
            token_budget: Some(90),
            working_turn_limit: DEFAULT_WORKING_TURN_LIMIT,
        });

        assert!(assembly
            .system_prompt
            .contains("Tier 2 Dynamic Working Context"));
        assert!(assembly.system_prompt.contains("[truncated]"));
        assert!(assembly.estimated_tokens <= 90);
        assert_eq!(assembly.messages[0].content, "latest request");
    }

    #[test]
    fn latest_user_message_is_never_truncated_to_empty() {
        let assembly = assemble_context(ContextAssemblyRequest {
            static_core_blocks: vec![ContextBlock::new("Core", "system ".repeat(100))],
            working_context_blocks: Vec::new(),
            working_messages: vec![message("user", "What is going on there?")],
            long_term_blocks: Vec::new(),
            token_budget: Some(1),
            working_turn_limit: DEFAULT_WORKING_TURN_LIMIT,
        });

        assert_eq!(assembly.messages.len(), 1);
        assert_eq!(assembly.messages[0].content, "What is going on there?");
    }

    #[test]
    fn oversized_current_turn_preserves_every_attachment_fairly() {
        let mut latest = message("user", "Compare Rust and Node releases.");
        latest.attachments = vec![
            attachment("local_web_search.md", &"rust evidence ".repeat(2_000)),
            attachment("local_web_search_2.md", &"node evidence ".repeat(2_000)),
        ];

        let assembly = assemble_context(ContextAssemblyRequest {
            static_core_blocks: vec![ContextBlock::new("Core", "system")],
            working_context_blocks: Vec::new(),
            working_messages: vec![latest],
            long_term_blocks: Vec::new(),
            token_budget: Some(1_000),
            working_turn_limit: DEFAULT_WORKING_TURN_LIMIT,
        });

        let selected = &assembly.messages[0];
        assert_eq!(selected.content, "Compare Rust and Node releases.");
        assert!(selected.attachments[0]
            .text
            .as_deref()
            .is_some_and(|text| text.contains("rust evidence")));
        assert!(selected.attachments[1]
            .text
            .as_deref()
            .is_some_and(|text| text.contains("node evidence")));
        let first_tokens = estimate_attachment_tokens(&selected.attachments[0]);
        let second_tokens = estimate_attachment_tokens(&selected.attachments[1]);
        assert!(second_tokens >= first_tokens);
        assert!(second_tokens.saturating_sub(first_tokens) <= 32);
        assert!(assembly.latest_user_message_condensed);
        assert!(assembly.latest_user_attachments_preserved);
    }
}
