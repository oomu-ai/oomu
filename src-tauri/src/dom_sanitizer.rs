use scraper::ElementRef;
use std::collections::HashSet;

const BOILERPLATE_TAGS: [&str; 3] = ["header", "nav", "footer"];
const BOILERPLATE_ROLES: [&str; 3] = ["banner", "navigation", "contentinfo"];
const BOILERPLATE_TOKENS: [&str; 9] = [
    "advertisement",
    "consent",
    "cookie",
    "gdpr",
    "newsletter",
    "notification",
    "promo",
    "promotional",
    "subscribe",
];

pub(crate) fn element_is_boilerplate(element: ElementRef<'_>) -> bool {
    std::iter::once(element)
        .chain(element.ancestors().filter_map(ElementRef::wrap))
        .any(|ancestor| {
            if BOILERPLATE_TAGS.contains(&ancestor.value().name()) {
                return true;
            }
            if ancestor.value().attr("role").is_some_and(|role| {
                BOILERPLATE_ROLES.contains(&role.trim().to_ascii_lowercase().as_str())
            }) {
                return true;
            }
            let hints = [ancestor.value().attr("id"), ancestor.value().attr("class")]
                .into_iter()
                .flatten()
                .flat_map(attribute_tokens)
                .collect::<HashSet<_>>();
            BOILERPLATE_TOKENS
                .iter()
                .any(|token| hints.contains(*token))
        })
}

pub(crate) fn semantic_markdown_block(element: ElementRef<'_>, text: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        return String::new();
    }
    match element.value().name() {
        "h1" => format!("# {text}"),
        "h2" => format!("## {text}"),
        "h3" => format!("### {text}"),
        "h4" | "h5" | "h6" => format!("#### {text}"),
        "li" | "dt" | "dd" => format!("- {text}"),
        "blockquote" => format!("> {text}"),
        "pre" => format!("```\n{text}\n```"),
        _ if element
            .value()
            .attr("role")
            .is_some_and(|role| role.eq_ignore_ascii_case("heading")) =>
        {
            format!("## {text}")
        }
        _ => text.to_string(),
    }
}

fn attribute_tokens(value: &str) -> impl Iterator<Item = String> + '_ {
    value
        .split(|character: char| !character.is_ascii_alphanumeric())
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_ascii_lowercase)
}
