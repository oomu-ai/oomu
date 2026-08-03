use super::ChatAttachment;
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::HashSet;

pub(super) const METADATA_KEY: &str = "publicGroundingProvenance";
const MAX_VISIBLE_SOURCES: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PublicGroundingProvenance {
    pub(super) url: String,
    pub(super) accessed_at_utc: String,
}

/// Projects only evidence that the native search pipeline actually opened.
/// Search-result snippets are intentionally excluded because their target pages
/// may not have been retrieved or inspected during the turn.
pub(super) fn from_attachments(attachments: &[ChatAttachment]) -> Vec<PublicGroundingProvenance> {
    let groups = source_groups(attachments);

    let mut seen_urls = HashSet::new();
    let mut provenance = Vec::new();
    let mut offsets = vec![0; groups.len()];
    loop {
        let mut progressed = false;
        for (group_index, group) in groups.iter().enumerate() {
            while let Some(source) = group.get(offsets[group_index]).cloned() {
                offsets[group_index] += 1;
                if !seen_urls.insert(source.url.clone()) {
                    continue;
                }
                provenance.push(source);
                progressed = true;
                break;
            }
            if provenance.len() == MAX_VISIBLE_SOURCES {
                return provenance;
            }
        }
        if !progressed {
            return provenance;
        }
    }
}

pub(super) fn all_source_urls(attachments: &[ChatAttachment]) -> HashSet<String> {
    source_groups(attachments)
        .into_iter()
        .flatten()
        .map(|source| source.url)
        .collect()
}

fn source_groups(attachments: &[ChatAttachment]) -> Vec<Vec<PublicGroundingProvenance>> {
    attachments
        .iter()
        .filter(|attachment| is_native_search_attachment(&attachment.name))
        .filter_map(|attachment| attachment.text.as_deref().and_then(search_context_json))
        .map(crate::sovereign_search::verified_sources::from_context_json)
        .filter(|sources| !sources.is_empty())
        .map(|sources| {
            sources
                .into_iter()
                .map(|source| PublicGroundingProvenance {
                    url: source.url,
                    accessed_at_utc: source.accessed_at_utc,
                })
                .collect()
        })
        .collect()
}

fn is_native_search_attachment(name: &str) -> bool {
    let normalized = name.trim().to_ascii_lowercase();
    if normalized == "local_web_search.md" {
        return true;
    }
    normalized
        .strip_prefix("local_web_search_")
        .and_then(|suffix| suffix.strip_suffix(".md"))
        .and_then(|index| index.parse::<usize>().ok())
        .is_some_and(|index| index >= 2)
}

pub(super) fn project_metadata(attachments: &[ChatAttachment], metadata: &mut Map<String, Value>) {
    let provenance = from_attachments(attachments);
    if !provenance.is_empty() {
        metadata.insert(
            METADATA_KEY.to_string(),
            serde_json::to_value(provenance).unwrap_or_else(|_| Value::Array(Vec::new())),
        );
    }
}

fn search_context_json(text: &str) -> Option<&str> {
    let json_start = text.find('{')?;
    Some(text.get(json_start..)?.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attachment(context: &str) -> ChatAttachment {
        let text = format!(
            "Local Web Search Context\nQuery: official source\nEngine: duckduckgo_lite_static\n\n{context}"
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
    fn projects_exact_native_access_time_and_retrieved_page_urls() {
        let context = serde_json::json!({
            "accessedAtUtc": "2026-07-23T14:12:13.456Z",
            "results": [{"url": "https://snippet-only.example/result"}],
            "pages": [
                {"url": "https://www.eia.gov/petroleum/gasdiesel/"},
                {"url": "https://www.eia.gov/petroleum/gasdiesel/"},
                {"url": "https://www.transportation.gov/brief"}
            ]
        });

        assert_eq!(
            from_attachments(&[attachment(&context.to_string())]),
            vec![
                PublicGroundingProvenance {
                    url: "https://www.eia.gov/petroleum/gasdiesel/".to_string(),
                    accessed_at_utc: "2026-07-23T14:12:13.456Z".to_string(),
                },
                PublicGroundingProvenance {
                    url: "https://www.transportation.gov/brief".to_string(),
                    accessed_at_utc: "2026-07-23T14:12:13.456Z".to_string(),
                },
            ]
        );
    }

    #[test]
    fn projects_retrieved_sources_from_each_bounded_search_attachment() {
        let first = serde_json::json!({
            "accessedAtUtc": "2026-07-23T14:12:13.456Z",
            "pages": [{"url": "https://www.rust-lang.org/"}]
        });
        let second = serde_json::json!({
            "accessedAtUtc": "2026-07-23T14:12:16.789Z",
            "pages": [{"url": "https://nodejs.org/en/download"}]
        });
        let mut second_attachment = attachment(&second.to_string());
        second_attachment.name = "local_web_search_2.md".to_string();

        assert_eq!(
            from_attachments(&[attachment(&first.to_string()), second_attachment]),
            vec![
                PublicGroundingProvenance {
                    url: "https://www.rust-lang.org/".to_string(),
                    accessed_at_utc: "2026-07-23T14:12:13.456Z".to_string(),
                },
                PublicGroundingProvenance {
                    url: "https://nodejs.org/en/download".to_string(),
                    accessed_at_utc: "2026-07-23T14:12:16.789Z".to_string(),
                },
            ]
        );
    }

    #[test]
    fn refuses_model_authored_or_malformed_provenance() {
        let vague_time = serde_json::json!({
            "accessedAtUtc": "Current Turn",
            "pages": [{"url": "https://www.eia.gov/petroleum/gasdiesel/"}]
        });
        let unsafe_url = serde_json::json!({
            "accessedAtUtc": "2026-07-23T14:12:13.456Z",
            "pages": [{"url": "file:///private/etc/passwd"}]
        });

        assert!(from_attachments(&[attachment(&vague_time.to_string())]).is_empty());
        assert!(from_attachments(&[attachment(&unsafe_url.to_string())]).is_empty());
        assert!(from_attachments(&[ChatAttachment {
            name: "notes.md".to_string(),
            ..attachment(&unsafe_url.to_string())
        }])
        .is_empty());
        assert!(from_attachments(&[ChatAttachment {
            name: "local_web_search_summary.md".to_string(),
            ..attachment(
                &serde_json::json!({
                    "accessedAtUtc": "2026-07-23T14:12:13.456Z",
                    "pages": [{"url": "https://model-authored.example/"}]
                })
                .to_string()
            )
        }])
        .is_empty());
    }

    #[test]
    fn all_retrieved_pages_remain_eligible_while_visible_sources_stay_bounded() {
        let pages = (1..=7)
            .map(|index| serde_json::json!({"url": format!("https://source-{index}.example/")}))
            .collect::<Vec<_>>();
        let context = serde_json::json!({
            "accessedAtUtc": "2026-07-23T14:12:13.456Z",
            "pages": pages,
        });
        let attachments = [attachment(&context.to_string())];

        assert_eq!(all_source_urls(&attachments).len(), 7);
        assert_eq!(from_attachments(&attachments).len(), MAX_VISIBLE_SOURCES);
    }
}
