use super::{public_grounding_provenance, ChatAttachment};
use regex::Regex;
use std::sync::OnceLock;

const PRESENTATION_DELIMITERS: &[char] = &[',', '.', ';', ':', '!', '?', ')', ']', '}'];

pub(super) fn contains_unverified_url(response: &str, attachments: &[ChatAttachment]) -> bool {
    static HTTP_URL: OnceLock<Regex> = OnceLock::new();
    let pattern = HTTP_URL.get_or_init(|| {
        Regex::new(r#"(?i)https?://[^\s<>\"'`]+"#).expect("valid grounded URL candidate regex")
    });
    let allowed = public_grounding_provenance::all_source_urls(attachments);
    if allowed.is_empty() {
        return false;
    }

    pattern.find_iter(response).any(|candidate| {
        let candidate = candidate.as_str();
        if allowed.contains(candidate) {
            return false;
        }
        !allowed.iter().any(|source| {
            candidate.strip_prefix(source).is_some_and(|suffix| {
                !suffix.is_empty()
                    && suffix
                        .chars()
                        .all(|character| PRESENTATION_DELIMITERS.contains(&character))
            })
        })
    })
}

/// Repairs only a presentation-equivalent trailing-slash variation by copying
/// the exact URL from native retrieval provenance. It does not authorize new
/// hosts, paths, queries, fragments, schemes, or model-authored sources; the
/// strict membership validator still runs after this normalization.
pub(super) fn canonicalize_verified_url_variants(
    response: &str,
    attachments: &[ChatAttachment],
) -> String {
    static HTTP_URL: OnceLock<Regex> = OnceLock::new();
    let pattern = HTTP_URL.get_or_init(|| {
        Regex::new(r#"(?i)https?://[^\s<>\"'`]+"#).expect("valid grounded URL candidate regex")
    });
    let mut allowed = public_grounding_provenance::all_source_urls(attachments)
        .into_iter()
        .collect::<Vec<_>>();
    allowed.sort();
    if allowed.is_empty() {
        return response.to_string();
    }

    pattern
        .replace_all(response, |captures: &regex::Captures<'_>| {
            canonical_url_variant(captures.get(0).map_or("", |value| value.as_str()), &allowed)
        })
        .into_owned()
}

fn canonical_url_variant(candidate: &str, allowed: &[String]) -> String {
    if allowed.iter().any(|source| {
        candidate == source
            || candidate.strip_prefix(source).is_some_and(|suffix| {
                !suffix.is_empty()
                    && suffix
                        .chars()
                        .all(|character| PRESENTATION_DELIMITERS.contains(&character))
            })
    }) {
        return candidate.to_string();
    }

    let mut core = candidate;
    while core
        .chars()
        .last()
        .is_some_and(|character| PRESENTATION_DELIMITERS.contains(&character))
    {
        core = &core[..core.len() - core.chars().last().map(char::len_utf8).unwrap_or(0)];
    }
    let suffix = &candidate[core.len()..];
    let canonical = allowed.iter().find(|source| {
        core.strip_suffix('/') == Some(source.as_str()) || source.strip_suffix('/') == Some(core)
    });
    canonical
        .map(|source| format!("{source}{suffix}"))
        .unwrap_or_else(|| candidate.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attachment(urls: &[&str]) -> ChatAttachment {
        let context = serde_json::json!({
            "accessedAtUtc": "2026-07-24T12:00:00.000Z",
            "pages": urls.iter().map(|url| serde_json::json!({"url": url})).collect::<Vec<_>>()
        });
        let text = format!(
            "Local Web Search Context\nQuery: official release\n\n{}",
            context
        );
        ChatAttachment {
            name: "local_web_search.md".to_string(),
            mime_type: "text/markdown".to_string(),
            byte_count: text.len(),
            data_base64: None,
            text: Some(text),
            approved_file_receipt: None,
        }
    }

    #[test]
    fn exact_native_urls_pass_in_plain_markdown_and_angle_forms() {
        let attachments = [attachment(&["https://nodejs.org/en/download"])];
        for response in [
            "Source: https://nodejs.org/en/download",
            "[Node.js](https://nodejs.org/en/download)",
            "<https://nodejs.org/en/download>",
        ] {
            assert!(
                !contains_unverified_url(response, &attachments),
                "{response}"
            );
        }
        assert!(!contains_unverified_url(
            "No URL in this grounded answer.",
            &attachments
        ));
        assert!(!contains_unverified_url(
            "https://active-page.example/",
            &[]
        ));
    }

    #[test]
    fn mutations_and_unopened_results_fail_exact_membership() {
        let attachments = [attachment(&["https://nodejs.org/en/download"])];
        for response in [
            "https://nodejs.org/en",
            "https://nodejs.org/en/download/",
            "https://nodejs.org/en/download?ref=answer",
            "https://nodejs.org/en/download#current",
            "https://NODEJS.org/en/download",
            "https://nodejs.org:443/en/download",
            "http://nodejs.org/en/download",
            "https://nodejs.org/en/download.attacker.example",
            "https://result-only.example/release",
        ] {
            assert!(
                contains_unverified_url(response, &attachments),
                "{response}"
            );
        }
    }

    #[test]
    fn legitimate_url_terminal_punctuation_is_not_blindly_truncated() {
        let attachments = [attachment(&[
            "https://example.com/release)",
            "https://example.com/version.",
        ])];
        assert!(!contains_unverified_url(
            "[source](https://example.com/release))",
            &attachments,
        ));
        assert!(!contains_unverified_url(
            "Exact source: https://example.com/version.",
            &attachments,
        ));
    }

    #[test]
    fn trailing_slash_variants_are_replaced_with_exact_retrieved_urls() {
        let attachments = [attachment(&[
            "https://doc.rust-lang.org/stable/releases.html",
            "https://blog.rust-lang.org/2026/07/16/Rust-1.97.1/",
        ])];
        let response = canonicalize_verified_url_variants(
            "[Index](https://doc.rust-lang.org/stable/releases.html/). [Post](https://blog.rust-lang.org/2026/07/16/Rust-1.97.1).",
            &attachments,
        );

        assert_eq!(
            response,
            "[Index](https://doc.rust-lang.org/stable/releases.html). [Post](https://blog.rust-lang.org/2026/07/16/Rust-1.97.1/)."
        );
        assert!(!contains_unverified_url(&response, &attachments));
    }

    #[test]
    fn canonicalization_never_rewrites_unverified_hosts_paths_or_queries() {
        let attachments = [attachment(&[
            "https://doc.rust-lang.org/stable/releases.html",
        ])];
        for response in [
            "https://attacker.example/stable/releases.html/",
            "https://doc.rust-lang.org/book/",
            "https://doc.rust-lang.org/stable/releases.html?ref=answer",
            "http://doc.rust-lang.org/stable/releases.html/",
        ] {
            assert_eq!(
                canonicalize_verified_url_variants(response, &attachments),
                response
            );
            assert!(contains_unverified_url(response, &attachments));
        }
    }
}
