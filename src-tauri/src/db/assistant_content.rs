use regex::Regex;
use std::sync::LazyLock;

const INTERNAL_ENVELOPE_NAMES: &[&str] = &[
    "tool_call",
    "tool_result",
    "native_receipt",
    "execution_receipt",
    "internal_directive",
    "oomu_control",
    "mcp_call",
    "function_call",
];

struct EnvelopePatterns {
    complete: Regex,
    unclosed: Regex,
    orphan: Regex,
}

static INTERNAL_ENVELOPE_PATTERNS: LazyLock<Vec<EnvelopePatterns>> = LazyLock::new(|| {
    INTERNAL_ENVELOPE_NAMES
        .iter()
        .map(|name| EnvelopePatterns {
            complete: Regex::new(&format!(r"(?is)<\s*{name}\b[^>]*>.*?<\s*/\s*{name}\s*>"))
                .expect("recognized assistant envelope pattern"),
            unclosed: Regex::new(&format!(r"(?is)<\s*{name}\b[^>]*>.*\z"))
                .expect("recognized unclosed assistant envelope pattern"),
            orphan: Regex::new(&format!(r"(?is)<\s*/?\s*{name}\b[^>]*>"))
                .expect("recognized orphan assistant envelope pattern"),
        })
        .collect()
});

pub(super) fn canonicalize_assistant_content(content: &str) -> String {
    let mut canonical = String::with_capacity(content.len());
    let mut cursor = 0;

    while let Some((fence_start, marker)) = next_fence(content, cursor) {
        let Some(opening_line_end) = content[fence_start..].find('\n') else {
            break;
        };
        let protected_start = fence_start + opening_line_end + 1;
        let Some(closing_offset) = content[protected_start..].find(marker) else {
            break;
        };
        let fence_end = protected_start + closing_offset + marker.len();
        canonical.push_str(&strip_internal_envelopes(&content[cursor..fence_start]));
        canonical.push_str(&content[fence_start..fence_end]);
        cursor = fence_end;
    }

    canonical.push_str(&strip_internal_envelopes(&content[cursor..]));
    canonical.trim().to_string()
}

fn next_fence(content: &str, cursor: usize) -> Option<(usize, &'static str)> {
    let remaining = content.get(cursor..)?;
    let backticks = remaining.find("```").map(|offset| (cursor + offset, "```"));
    let tildes = remaining.find("~~~").map(|offset| (cursor + offset, "~~~"));
    match (backticks, tildes) {
        (Some(left), Some(right)) => Some(if left.0 <= right.0 { left } else { right }),
        (Some(fence), None) | (None, Some(fence)) => Some(fence),
        (None, None) => None,
    }
}

fn strip_internal_envelopes(content: &str) -> String {
    let mut cleaned = content.to_string();
    for patterns in INTERNAL_ENVELOPE_PATTERNS.iter() {
        cleaned = patterns.complete.replace_all(&cleaned, "").into_owned();
        cleaned = patterns.unclosed.replace_all(&cleaned, "").into_owned();
        cleaned = patterns.orphan.replace_all(&cleaned, "").into_owned();
    }
    while cleaned.contains("\n\n\n") {
        cleaned = cleaned.replace("\n\n\n", "\n\n");
    }
    cleaned
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_complete_orphan_and_unclosed_internal_envelopes() {
        let content = concat!(
            "Visible result.\n",
            "<tool_call>{\"name\":\"read_file\"}</tool_call>\n",
            "Still visible.</tool_result>\n",
            "<native_receipt>{\"verified\":true}"
        );
        assert_eq!(
            canonicalize_assistant_content(content),
            "Visible result.\n\nStill visible."
        );
    }

    #[test]
    fn preserves_complete_fenced_literal_examples_exactly() {
        let content = concat!(
            "Example:\n```xml\n",
            "<tool_call>{\"literal\":true}</tool_call>\n",
            "<tool_result>literal result</tool_result>\n",
            "```\nDone."
        );
        assert_eq!(canonicalize_assistant_content(content), content);
    }
}
