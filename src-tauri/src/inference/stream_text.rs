use crate::gemma::sanitize_gemma4_response;
use serde_json::Value;

pub(super) fn sanitize_stream_text(value: &str) -> String {
    let unwrapped = unwrap_serialized_protocol_stream(value).unwrap_or_else(|| value.to_string());
    let candidate = unwrapped.trim_start_matches('\u{feff}').trim_start();
    let extracted = if looks_like_protocol_stream(candidate) {
        merge_stream_text_chunks(candidate.lines().filter_map(protocol_text_chunk))
    } else {
        unwrapped
    };
    sanitize_gemma4_response(&extracted)
}

fn protocol_text_chunk(line: &str) -> Option<String> {
    let mut record = line.trim();
    if record.is_empty() || record.starts_with(':') || record.eq_ignore_ascii_case("[DONE]") {
        return None;
    }
    if record
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("data:"))
    {
        record = record[5..].strip_prefix(' ').unwrap_or(&record[5..]);
    }
    let (prefix, payload) = record.split_once(':')?;
    if prefix != "0" {
        return None;
    }
    serde_json::from_str::<Value>(payload)
        .ok()
        .and_then(|decoded| decoded.as_str().map(ToString::to_string))
}

pub(super) fn merge_stream_text_chunks(chunks: impl IntoIterator<Item = String>) -> String {
    let mut merged = String::new();
    for chunk in chunks {
        let text = merge_stream_text_chunk(&merged, &chunk);
        merged.push_str(&text);
    }
    merged
}

pub(super) fn merge_stream_text_chunk(previous: &str, chunk: &str) -> String {
    if chunk.is_empty() {
        return String::new();
    }
    let mut text = String::new();
    if needs_stream_chunk_boundary_space(previous, chunk) {
        text.push(' ');
    }
    text.push_str(chunk);
    text
}

fn needs_stream_chunk_boundary_space(previous: &str, next: &str) -> bool {
    let Some(previous_char) = previous.chars().next_back() else {
        return false;
    };
    let Some(next_char) = next.chars().next() else {
        return false;
    };
    if previous_char.is_whitespace() || next_char.is_whitespace() {
        return false;
    }
    if ends_with_unclosed_protocol_marker(previous) {
        return false;
    }
    if is_sentence_terminal(previous_char) && next_char.is_alphanumeric() {
        return true;
    }
    if previous_char.is_alphabetic() && next_char.is_alphabetic() {
        return looks_like_missing_word_boundary(previous, next);
    }
    false
}

fn looks_like_missing_word_boundary(previous: &str, next: &str) -> bool {
    let previous_word = trailing_alphabetic_word(previous);
    let next_word = leading_alphabetic_word(next);
    if previous_word.is_empty() || next_word.is_empty() {
        return false;
    }
    let previous_word = previous_word.to_lowercase();
    let next_word = next_word.to_lowercase();
    if is_continuation_fragment(&next_word) {
        return false;
    }
    if is_boundary_standalone_word(&previous_word) || is_boundary_standalone_word(&next_word) {
        return true;
    }
    previous_word.chars().count() >= 4 && next_word.chars().count() >= 4
}

fn trailing_alphabetic_word(value: &str) -> String {
    let mut word = value
        .chars()
        .rev()
        .take_while(|character| character.is_alphabetic())
        .collect::<Vec<_>>();
    word.reverse();
    word.into_iter().collect()
}

fn leading_alphabetic_word(value: &str) -> String {
    value
        .chars()
        .take_while(|character| character.is_alphabetic())
        .collect()
}

fn is_boundary_standalone_word(value: &str) -> bool {
    matches!(
        value,
        "a" | "about"
            | "after"
            | "almost"
            | "also"
            | "am"
            | "an"
            | "and"
            | "are"
            | "as"
            | "at"
            | "be"
            | "because"
            | "been"
            | "before"
            | "between"
            | "but"
            | "by"
            | "can"
            | "could"
            | "did"
            | "do"
            | "does"
            | "for"
            | "from"
            | "go"
            | "had"
            | "has"
            | "have"
            | "he"
            | "her"
            | "here"
            | "his"
            | "how"
            | "i"
            | "if"
            | "in"
            | "into"
            | "is"
            | "it"
            | "its"
            | "leading"
            | "me"
            | "might"
            | "my"
            | "not"
            | "now"
            | "of"
            | "on"
            | "or"
            | "our"
            | "she"
            | "should"
            | "spacing"
            | "still"
            | "that"
            | "the"
            | "their"
            | "then"
            | "there"
            | "these"
            | "they"
            | "this"
            | "those"
            | "to"
            | "tokenization"
            | "under"
            | "was"
            | "we"
            | "were"
            | "what"
            | "when"
            | "where"
            | "which"
            | "while"
            | "who"
            | "will"
            | "with"
            | "world"
            | "would"
            | "you"
            | "your"
    )
}

fn is_continuation_fragment(value: &str) -> bool {
    matches!(
        value,
        "al" | "ally"
            | "ation"
            | "ations"
            | "ary"
            | "ed"
            | "er"
            | "ers"
            | "es"
            | "est"
            | "figuration"
            | "ful"
            | "ible"
            | "ing"
            | "ingly"
            | "ization"
            | "izations"
            | "ize"
            | "ized"
            | "izes"
            | "izing"
            | "less"
            | "ly"
            | "ment"
            | "ments"
            | "ness"
            | "ory"
            | "ous"
            | "ously"
            | "pletely"
            | "s"
            | "sion"
            | "sions"
            | "tion"
            | "tions"
            | "uration"
            | "ure"
            | "ures"
    )
}

fn ends_with_unclosed_protocol_marker(value: &str) -> bool {
    let Some(start) = value.rfind('<') else {
        return false;
    };
    !value[start..].contains('>')
}

fn is_sentence_terminal(character: char) -> bool {
    matches!(
        character,
        '.' | ',' | '!' | '?' | ';' | ':' | ')' | ']' | '}' | '"' | '\''
    )
}

fn unwrap_serialized_protocol_stream(value: &str) -> Option<String> {
    let parsed = serde_json::from_str::<Value>(value.trim()).ok()?;
    match parsed {
        Value::String(text) if looks_like_protocol_stream(text.trim_start()) => Some(text),
        Value::Array(entries)
            if entries.iter().all(Value::is_string)
                && entries.iter().any(|entry| {
                    entry
                        .as_str()
                        .is_some_and(|text| looks_like_protocol_stream(text.trim_start()))
                }) =>
        {
            Some(
                entries
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
        }
        _ => None,
    }
}

fn looks_like_protocol_stream(value: &str) -> bool {
    let candidate = value.trim_start();
    let without_data = candidate
        .strip_prefix("data:")
        .map(str::trim_start)
        .unwrap_or(candidate);
    without_data.split_once(':').is_some_and(|(prefix, _)| {
        prefix.len() == 1 && prefix.chars().all(|ch| ch.is_ascii_alphanumeric())
    }) || candidate.starts_with("event:")
        || candidate.starts_with("id:")
        || candidate.starts_with("retry:")
        || candidate
            .get(..13)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("content-type:"))
        || candidate.starts_with(':')
}
