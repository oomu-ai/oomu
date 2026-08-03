use regex::Regex;
use std::{collections::HashSet, sync::OnceLock};

pub(crate) fn exact_response_for_prompt(prompt: &str) -> Option<&'static str> {
    static STORED_REPLY: OnceLock<Regex> = OnceLock::new();
    let prompt = latest_user_turn(prompt);
    STORED_REPLY
        .get_or_init(|| {
            Regex::new(
                r#"(?is)\b(?:this|the\s+current)\s+(?:chat|conversation|session)\s+only\b[\s\S]{0,240}\breply\s+(?:with\s+)?[\"'“”]?stored[\"'“”]?[.!]?\s*$"#,
            )
            .expect("bounded stored-reply contract regex is valid")
        })
        .is_match(prompt.trim())
        .then_some("stored")
}

pub(crate) fn bounded_rewrite_response(prompt: &str) -> Option<String> {
    static REWRITE: OnceLock<Regex> = OnceLock::new();
    let prompt = latest_user_turn(prompt);
    let captures = REWRITE
        .get_or_init(|| {
            Regex::new(
                r#"(?is)\breplace\s+every\s+occurrence\s+of\s+[`\"'“”]?([[:alnum:]_'-]+)[`\"'“”]?\s+with\s+[`\"'“”]?([[:alnum:]_'-]+)[`\"'“”]?\.\s*make\s+no\s+other\s+change\s+and\s+do\s+not\s+explain\.?\s*$"#,
            )
            .expect("bounded exact-rewrite contract regex is valid")
        })
        .captures(prompt)?;
    let directive = captures.get(0)?;
    let source_word = captures.get(1)?.as_str();
    let replacement_word = captures.get(2)?.as_str();
    let source = prompt[..directive.start()].trim_end();
    let occurrences = source.matches(source_word).count();
    (source.len() <= 64 * 1024 && (1..=512).contains(&occurrences))
        .then(|| source.replace(source_word, replacement_word))
}

fn latest_user_turn(prompt: &str) -> &str {
    if let Some(start) = prompt.rfind("<|turn>user\n") {
        let content = &prompt[start + "<|turn>user\n".len()..];
        return content
            .find("<turn|>")
            .map(|end| &content[..end])
            .unwrap_or(content)
            .trim();
    }
    if let Some(start) = prompt.rfind("\nUser: ") {
        let content = &prompt[start + "\nUser: ".len()..];
        return content
            .rfind("\nAssistant:")
            .map(|end| &content[..end])
            .unwrap_or(content)
            .trim();
    }
    prompt.trim()
}

fn split_view_block_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?is)<oomusplitview\b[^>]*>.*?</oomusplitview\s*>")
            .expect("split-view block regex is valid")
    })
}

fn split_view_tag_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?is)</?oomusplitview\b[^>]*>").expect("split-view tag regex is valid")
    })
}

pub(crate) fn strip_orphan_oomu_split_view_tags(response: &str) -> String {
    let mut sanitized = String::with_capacity(response.len());
    let mut cursor = 0;
    for complete_block in split_view_block_regex().find_iter(response) {
        sanitized.push_str(
            &split_view_tag_regex().replace_all(&response[cursor..complete_block.start()], ""),
        );
        sanitized.push_str(complete_block.as_str());
        cursor = complete_block.end();
    }
    sanitized.push_str(&split_view_tag_regex().replace_all(&response[cursor..], ""));
    sanitized.trim().to_string()
}

pub(crate) fn has_orphan_oomu_split_view_tag(response: &str) -> bool {
    split_view_tag_regex().find_iter(response).count()
        != split_view_block_regex()
            .find_iter(response)
            .count()
            .saturating_mul(2)
}

pub(crate) fn has_repetition_collapse(prompt: &str, text: &str) -> bool {
    let words = canonical_response_words(text);
    if words.len() < 20 {
        return false;
    }
    let unique_words = words.iter().collect::<HashSet<_>>();
    unique_words.len() as f64 / (words.len() as f64) < 0.25
        && !requested_rewrite_is_source_bounded(prompt, text)
}

pub(crate) fn requested_rewrite_is_source_bounded(prompt: &str, response: &str) -> bool {
    static REPLACEMENT: OnceLock<Regex> = OnceLock::new();
    let replacement = REPLACEMENT.get_or_init(|| {
        Regex::new(
            r#"(?i)\breplace\s+every\s+occurrence\s+of\s+['"`]?([[:alnum:]_'-]+)['"`]?\s+with\s+['"`]?([[:alnum:]_'-]+)"#,
        )
        .expect("bounded replacement directive regex is valid")
    });
    let Some(captures) = replacement.captures(prompt) else {
        return false;
    };
    let Some(source_word) = captures
        .get(1)
        .map(|value| value.as_str().to_ascii_lowercase())
    else {
        return false;
    };
    let Some(replacement_word) = captures
        .get(2)
        .map(|value| value.as_str().to_ascii_lowercase())
    else {
        return false;
    };

    let response_words = canonical_response_words(response);
    if response_words.len() < 20 {
        return false;
    }
    let transformed_prompt_words = canonical_response_words(prompt)
        .into_iter()
        .map(|word| {
            if word == source_word {
                replacement_word.clone()
            } else {
                word
            }
        })
        .collect::<Vec<_>>();

    transformed_prompt_words
        .windows(response_words.len())
        .any(|window| window == response_words.as_slice())
}

fn canonical_response_words(value: &str) -> Vec<String> {
    value
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '\'')
        .filter(|word| !word.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}
