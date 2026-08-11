use super::{authorization_policy, clean_search_topic, continuation, strip_search_courtesy_prefix};
use regex::Regex;

pub(super) fn explicit_search_query_from_utterance(utterance: &str) -> Option<String> {
    if let Some(topic) = continuation::authorized_browser_research_query(utterance) {
        return Some(topic);
    }
    if let Some(topic) = authorization_policy::localized_explicit_search_query(utterance) {
        return Some(topic);
    }
    for raw_clause in std::iter::once(utterance).chain(utterance.split(['!', '?', ';', '\n'])) {
        for raw_sentence in std::iter::once(raw_clause).chain(raw_clause.split(". ")) {
            let clause = strip_search_courtesy_prefix(&clean_search_topic(raw_sentence));
            if clause.is_empty() {
                continue;
            }
            if Regex::new(r"(?i)^check\s+google\s+calendar\b")
                .expect("static Google Calendar exclusion regex")
                .is_match(&clause)
            {
                continue;
            }
            for pattern in [
                r"(?i)^go\s+(?:online|on\s+the\s+(?:web|internet))\s*,?\s+and\s+(?:research|search(?:\s+for)?|find(?:\s+out)?)\s*:?[ ]*(.+)$",
                r"(?i)^(?:search|browse|check|confirm|find|look|research|see|verify)\s+(?:(?:on|using)\s+)?(?:the\s+)?(?:public\s+)?(?:web|internet|online|google|duckduckgo)\s*(?:(?:for|about|on|regarding)\s+|(?:to|and)\s+(?:check|confirm|find|look(?:\s+up)?|research|search(?:\s+for)?|see|verify)\s+|(?:if|whether|that)\s+)?(.+?)(?:\.\s+(?:cite|include|provide|return|summarize|write|create|list|explain|then)\b.*)?$",
                r"(?i)^(?:take|have)\s+(?:(?:a|another)\s+)?(?:(?:careful|closer|fresh|proper|quick|thorough)\s+)?look(?:\s+around)?\s+(?:(?:on|using|across|around|through)\s+)?(?:the\s+)?(?:public\s+)?(?:web|internet|online|google|duckduckgo)\s*(?:(?:for|about|on|regarding)\s+)?(.+)$",
                r"(?i)^(?:do|run|conduct|perform)\s+(?:a\s+)?(?:(?:careful|fresh|quick|thorough)\s+)?(?:web|internet|online|google|duckduckgo)\s+(?:search|check|lookup|research)\s*(?:(?:for|about|on|regarding)\s+)?(.+)$",
                r"(?i)^see\s+what\s+(?:you\s+can\s+)?find\s+(?:(?:on|using)\s+)?(?:the\s+)?(?:public\s+)?(?:web|internet|online|google|duckduckgo)\s+(?:about|on|for|regarding)\s+(.+)$",
                r"(?i)^see\s+(?:if|whether)\s+(?:you\s+can\s+)?find\s+(.+?)\s+(?:(?:on|using)\s+)?(?:the\s+)?(?:public\s+)?(?:web|internet|online|google|duckduckgo)$",
                r"(?i)^use\s+(?:the\s+)?(?:public\s+)?(?:web|internet|online|google|duckduckgo)\s+to\s+(?:search|browse|check|confirm|find|look\s+up|research|see|verify)\s*(?:(?:for|about|on|regarding)\s+|(?:if|whether|that)\s+)?(.+?)(?:\.\s+(?:cite|include|provide|return|summarize|write|create|list|explain|then)\b.*)?$",
                r"(?i)^look\s+(.+?)\s+up\s+(?:(?:on|using)\s+)?(?:the\s+)?(?:public\s+)?(?:web|internet|online|google|duckduckgo)$",
                r"(?i)^look\s+up\s+(.+?)\s+(?:(?:on|using)\s+)?(?:the\s+)?(?:public\s+)?(?:web|internet|online|google|duckduckgo)$",
                r"(?i)^(?:search|browse|check|confirm|consult|explore|find|investigate|look|research|see|verify)\s+(?:for|about|on|regarding)\s+(.+?)\s+(?:(?:on|using|across)\s+)?(?:the\s+)?(?:public\s+)?(?:web|internet|online|google|duckduckgo)$",
                r"(?i)^research\s+(?:(?:current|recent|latest|public|primary|official|authoritative|web|or|and)\s+)+sources?\s+(?:(?:on|about|regarding|for)\s+|relevant\s+to\s+)(.+)$",
            ] {
                let regex = Regex::new(pattern).expect("static search binding regex is valid");
                if let Some(topic) = regex
                    .captures(&clause)
                    .and_then(|captures| captures.get(1))
                    .map(|capture| clean_search_topic(capture.as_str()))
                    .filter(|topic| !topic.is_empty())
                {
                    let topic = Regex::new(r"(?i)^what\s+you\s+can\s+find\s+about\s+")
                        .expect("static search topic prefix regex")
                        .replace(&topic, "")
                        .trim()
                        .to_string();
                    let topic = Regex::new(
                        r"(?i)^(?:to|and)\s+(?:check|confirm|find(?:\s+out)?|look(?:\s+up)?|research|search(?:\s+for)?|see|verify)\s+",
                    )
                    .expect("static search action prefix regex")
                    .replace(&topic, "")
                    .trim()
                    .to_string();
                    let topic = Regex::new(
                        r"(?i),?\s+then\s+(?:check|find|look|open|read|review|search|verify)\b.*$",
                    )
                    .expect("static dependent search suffix regex")
                    .replace(&topic, "")
                    .trim()
                    .to_string();
                    let topic = Regex::new(
                        r"(?i),?\s+and\s+(?:(?:give|provide|send|show|tell)\s+me|return(?:\s+me)?)\b.*$",
                    )
                    .expect("static search delivery suffix regex")
                    .replace(&topic, "")
                    .trim()
                    .to_string();
                    let topic = Regex::new(r"(?i)^(?:if|whether|that)\s+")
                        .expect("static search topic clause regex")
                        .replace(&topic, "")
                        .trim()
                        .to_string();
                    if !authorization_policy::search_topic_is_weak_or_deictic(&topic) {
                        return Some(topic);
                    }
                }
            }
        }
    }
    None
}
