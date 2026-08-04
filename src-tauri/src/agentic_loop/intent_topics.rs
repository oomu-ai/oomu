pub(super) fn is_informational_local_system_topic_question(prompt: &str) -> bool {
    let normalized = prompt.to_lowercase();
    let trimmed = normalized.trim();
    if trimmed.is_empty() || !mentions_local_system_topic(trimmed) {
        return false;
    }
    if [
        "check my",
        "read my",
        "review my",
        "scan my",
        "show my",
        "show me my",
        "list my",
        "find my",
        "look for",
        "summarize my",
        "summarise my",
        "report on my",
        "what is on my",
        "what's on my",
        "what are my",
        "what's in my",
        "what is in my",
        "do i have",
        "did i have",
        "are there",
        "how many",
        "when is my",
        "when are my",
    ]
    .iter()
    .any(|term| trimmed.contains(term))
    {
        return false;
    }
    [
        "how do i",
        "how can i",
        "how should i",
        "how does",
        "how do",
        "how ",
        "what is",
        "what are",
        "why does",
        "why do",
        "explain",
        "tell me about",
        "tell me how",
        "help me understand",
        "configure",
        "set up",
        "setup",
        "troubleshoot",
    ]
    .iter()
    .any(|term| trimmed.contains(term))
}

pub(super) fn mentions_local_system_topic(prompt: &str) -> bool {
    [
        "mail",
        "email",
        "e-mail",
        "inbox",
        "calendar",
        "agenda",
        "scheduled event",
        "reminder",
        "task",
        "todo",
        "to-do",
        "note",
        "contact",
        "address book",
        "photo",
        "picture",
        "camera roll",
        "messages app",
        "imessage",
    ]
    .iter()
    .any(|term| prompt.contains(term))
}
