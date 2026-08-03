use super::{
    fabricated_history_unavailable_claim, is_search_grounding_attachment,
    response_integrity_retry_reason, ChatAttachment, InferenceError, InferenceResponse,
};
use regex::Regex;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use url::Url;

const ZERO_MOCKERY_RETRY_WARNING: &str = "<system_error> The output contained an unresolved value. Re-evaluate using verified data, or state the exact missing input and whether any action occurred. </system_error>";
const RELEASE_DATE_FIELDS: &[&str] = &[
    "release date",
    "release dates",
    "date of release",
    "dates of release",
    "veröffentlichungsdatum",
    "veröffentlichungsdaten",
    "fecha de lanzamiento",
    "fechas de lanzamiento",
    "date de sortie",
    "dates de sortie",
    "tanggal rilis",
    "リリース日",
    "発売日",
    "data de lançamento",
    "datas de lançamento",
    "дата релиза",
    "даты релиза",
    "дату релиза",
    "дата выпуска",
    "даты выпуска",
    "дата релізу",
    "дати релізу",
    "дату релізу",
    "дата випуску",
    "дати випуску",
    "ngày phát hành",
    "发布日期",
    "发布日",
    "發佈日期",
    "發行日期",
    "發佈日",
];
const RELEASE_DATE_DEFICITS: &[&str] = &[
    "absent",
    "missing",
    "unavailable",
    "unknown",
    "unverified",
    "not provide",
    "no date",
    "fehlt",
    "fehlend",
    "nicht verfügbar",
    "nicht angegeben",
    "unbekannt",
    "unbestätigt",
    "falta",
    "no disponible",
    "no proporcion",
    "desconocid",
    "sin verificar",
    "manqu",
    "indisponible",
    "non fourni",
    "inconnu",
    "non vérifié",
    "tidak tersedia",
    "tidak diberikan",
    "tidak diketahui",
    "belum terverifikasi",
    "記載されていない",
    "提供されていない",
    "不明",
    "確認でき",
    "検証でき",
    "ausente",
    "não disponível",
    "não fornec",
    "desconhecid",
    "não verific",
    "отсутств",
    "недоступ",
    "не указ",
    "неизвест",
    "не подтверж",
    "відсут",
    "недоступ",
    "не вказ",
    "невідом",
    "не підтвердж",
    "thiếu",
    "không có",
    "không được cung cấp",
    "không xác minh",
    "chưa xác minh",
    "缺少",
    "未提供",
    "不可用",
    "未知",
    "无法验证",
    "未验证",
    "無法驗證",
    "未驗證",
];
const RELEASE_DATE_EVIDENCE: &[&str] = &[
    "source",
    "evidence",
    "official",
    "verified",
    "quelle",
    "beleg",
    "offiziell",
    "verifiziert",
    "bestätigt",
    "fuente",
    "evidencia",
    "oficial",
    "verificada",
    "preuve",
    "officiel",
    "vérifié",
    "sumber",
    "bukti",
    "resmi",
    "terverifikasi",
    "情報源",
    "出典",
    "公式",
    "検証",
    "確認",
    "fonte",
    "evidência",
    "verificada",
    "источник",
    "доказ",
    "официаль",
    "провер",
    "подтверж",
    "джерел",
    "офіцій",
    "перевір",
    "підтвердж",
    "nguồn",
    "bằng chứng",
    "chính thức",
    "xác minh",
    "来源",
    "來源",
    "證據",
    "证据",
    "官方",
    "驗證",
    "验证",
];

pub(super) fn prospective_search_promise(response: &str) -> bool {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(
        r"(?is)\b(?:(?:i|we)\s*(?:am|are|['’]m|['’]re|will|['’]ll|am\s+going\s+to)|let\s+me)\b.{0,72}\b(?:search|research|look\s+up|browse|go\s+online)\b|\b(?:issuing|starting|running|performing|conducting)\s+(?:the\s+|another\s+|a\s+)?(?:web\s+|online\s+|public\s+)?(?:search|research)\b",
    ).expect("valid prospective search promise regex")).is_match(response)
}

pub(super) fn grounded_browser_action_claim(response: &str) -> bool {
    static FIRST_PERSON: OnceLock<Regex> = OnceLock::new();
    static NARRATED: OnceLock<Regex> = OnceLock::new();
    FIRST_PERSON.get_or_init(|| Regex::new(
        r"(?is)\b(?:i|we)\s*(?:am|are|['’]m|['’]re|will|['’]ll)\b.{0,64}\b(?:launch|open|activate|start|bring\s+up|use)(?:ing)?\b.{0,120}\b(?:browser|web\s+panel|browser\s+panel|split\s+view|co-browsing)\b",
    ).expect("valid grounded browser claim regex")).is_match(response)
        || NARRATED.get_or_init(|| Regex::new(
            r"(?is)\b(?:launching|opening|activating|starting|bringing\s+up|using)\b.{0,100}\b(?:browser|web\s+panel|browser\s+panel|split\s+view|co-browsing)\b",
        ).expect("valid narrated browser claim regex")).is_match(response)
}

pub(super) fn grounded_output_violation(response: &str) -> bool {
    grounded_browser_action_claim(response) || prospective_search_promise(response)
}

fn grounded_capability_refusal(response: &str) -> bool {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN
        .get_or_init(|| {
            Regex::new(
                r"(?is)\b(?:i|we|oomu)\s+(?:do\s+not|don't|cannot|can't|am\s+unable\s+to|are\s+unable\s+to|lack)\b.{0,120}\b(?:real[- ]time|live|internet|web|online|brows(?:e|ing)|external\s+(?:sports\s+)?schedules?)\b",
            )
            .expect("valid grounded capability refusal regex")
        })
        .is_match(response)
}

pub(super) fn chat_response_retry_reason(
    response: &InferenceResponse,
    current_user_message: &str,
    verified_prior_conversation_available: bool,
    headless_grounding_active: bool,
    attachments: &[ChatAttachment],
) -> Option<&'static str> {
    if headless_grounding_active && prospective_search_promise(&response.text) {
        return Some("grounded_search_promise");
    }
    if headless_grounding_active && grounded_browser_action_claim(&response.text) {
        return Some("grounded_browser_action_claim");
    }
    if headless_grounding_active && grounded_capability_refusal(&response.text) {
        return Some("grounded_capability_refusal");
    }
    if headless_grounding_active && grounding_internal_context_leak(&response.text) {
        return Some("grounded_internal_context_leak");
    }
    if headless_grounding_active
        && super::grounded_citation_integrity::contains_unverified_url(&response.text, attachments)
    {
        return Some("grounded_unverified_citation");
    }
    if headless_grounding_active
        && grounded_nonofficial_citation_for_official_request(&response.text, current_user_message)
    {
        return Some("grounded_nonofficial_citation");
    }
    if headless_grounding_active
        && grounded_unsupported_material_intensifier(&response.text, attachments)
    {
        return Some("grounded_unsupported_intensifier");
    }
    if headless_grounding_active && grounded_objective_coverage_missing(&response.text, attachments)
    {
        return Some("grounded_objective_coverage");
    }
    if headless_grounding_active
        && grounded_requested_field_coverage_missing(
            &response.text,
            current_user_message,
            attachments,
        )
    {
        return Some("grounded_requested_field_coverage");
    }
    if fabricated_history_unavailable_claim(
        &response.text,
        current_user_message,
        verified_prior_conversation_available,
    ) {
        return Some("fabricated_history_unavailable");
    }
    response_integrity_retry_reason(response)
}

pub(super) fn is_grounded_repair_reason(reason: &str) -> bool {
    matches!(
        reason,
        "grounded_browser_action_claim"
            | "grounded_capability_refusal"
            | "grounded_search_promise"
            | "grounded_internal_context_leak"
            | "grounded_unverified_citation"
            | "grounded_nonofficial_citation"
            | "grounded_unsupported_intensifier"
            | "grounded_objective_coverage"
            | "grounded_requested_field_coverage"
    )
}

fn grounded_nonofficial_citation_for_official_request(
    response: &str,
    current_user_message: &str,
) -> bool {
    static OFFICIAL_REQUEST: OnceLock<Regex> = OnceLock::new();
    if !OFFICIAL_REQUEST
        .get_or_init(|| {
            Regex::new(r"(?i)\b(?:official|first[- ]party|vendor[- ](?:controlled|published)|publisher[- ]controlled)\b")
                .expect("valid official-source request regex")
        })
        .is_match(current_user_message)
    {
        return false;
    }

    let subject_tokens = official_subject_tokens(current_user_message);
    if subject_tokens.is_empty() {
        return false;
    }
    let cited_urls = grounded_response_urls(response);
    if cited_urls.len() < 2 {
        return false;
    }
    let authority_matches = cited_urls
        .iter()
        .map(|url| official_url_matches_subject(url, &subject_tokens))
        .collect::<Vec<_>>();

    authority_matches.iter().any(|matches| *matches)
        && authority_matches.iter().any(|matches| !*matches)
}

fn official_subject_tokens(message: &str) -> HashSet<String> {
    static WORD: OnceLock<Regex> = OnceLock::new();
    const GENERIC: &[&str] = &[
        "about",
        "after",
        "also",
        "answer",
        "check",
        "could",
        "current",
        "date",
        "decide",
        "direct",
        "does",
        "exact",
        "example",
        "features",
        "find",
        "first",
        "give",
        "includes",
        "language",
        "latest",
        "links",
        "look",
        "newly",
        "notes",
        "official",
        "online",
        "page",
        "pages",
        "publisher",
        "recommendation",
        "release",
        "releases",
        "right",
        "short",
        "source",
        "sources",
        "stable",
        "stabilized",
        "tell",
        "their",
        "then",
        "there",
        "these",
        "third",
        "trying",
        "update",
        "updating",
        "used",
        "vendor",
        "version",
        "whether",
        "with",
        "worth",
        "would",
        "your",
    ];
    WORD.get_or_init(|| {
        Regex::new(r"(?i)[a-z0-9][a-z0-9.+-]{2,}").expect("valid subject token regex")
    })
    .find_iter(message)
    .map(|candidate| candidate.as_str().to_ascii_lowercase())
    .filter(|token| token.len() >= 4 && !GENERIC.contains(&token.as_str()))
    .collect()
}

fn grounded_response_urls(response: &str) -> Vec<Url> {
    static HTTP_URL: OnceLock<Regex> = OnceLock::new();
    HTTP_URL
        .get_or_init(|| {
            Regex::new(r#"(?i)https?://[^\s<>\"'`]+"#).expect("valid grounded response URL regex")
        })
        .find_iter(response)
        .filter_map(|candidate| {
            let value = candidate.as_str().trim_end_matches(|character| {
                matches!(
                    character,
                    '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}'
                )
            });
            Url::parse(value).ok()
        })
        .collect()
}

fn official_url_matches_subject(url: &Url, subject_tokens: &HashSet<String>) -> bool {
    let identity =
        format!("{}{}", url.host_str().unwrap_or_default(), url.path()).to_ascii_lowercase();
    subject_tokens.iter().any(|token| identity.contains(token))
}

fn grounded_unsupported_material_intensifier(
    response: &str,
    attachments: &[ChatAttachment],
) -> bool {
    let evidence = grounded_evidence_lowercase(attachments);
    if evidence.is_empty() {
        return false;
    }

    material_intensifier_regex()
        .find_iter(response)
        .any(|candidate| unsupported_material_intensifier(response, &candidate, &evidence))
}

pub(super) fn neutralize_unsupported_material_intensifiers(
    response: &str,
    attachments: &[ChatAttachment],
) -> Option<String> {
    let evidence = grounded_evidence_lowercase(attachments);
    if evidence.is_empty() {
        return None;
    }

    let mut sanitized = String::with_capacity(response.len());
    let mut cursor = 0usize;
    let mut changed = false;
    for candidate in material_intensifier_regex().find_iter(response) {
        if !unsupported_material_intensifier(response, &candidate, &evidence) {
            continue;
        }
        sanitized.push_str(&response[cursor..candidate.start()]);
        sanitized.push_str("documented");
        cursor = candidate.end();
        changed = true;
    }
    if !changed {
        return None;
    }
    sanitized.push_str(&response[cursor..]);
    Some(sanitized)
}

fn material_intensifier_regex() -> &'static Regex {
    static INTENSIFIER: OnceLock<Regex> = OnceLock::new();
    INTENSIFIER.get_or_init(|| {
        Regex::new(r"(?i)\b(?:security[- ]critical|high[- ]severity|critical|catastrophic)\b")
            .expect("valid grounded intensifier regex")
    })
}

fn grounded_evidence_lowercase(attachments: &[ChatAttachment]) -> String {
    attachments
        .iter()
        .filter_map(|attachment| attachment.text.as_deref())
        .filter(|text| is_search_grounding_attachment_from_text(text))
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase()
}

fn unsupported_material_intensifier(
    response: &str,
    candidate: &regex::Match<'_>,
    evidence: &str,
) -> bool {
    let phrase = candidate.as_str().to_ascii_lowercase();
    let prefix = response[..candidate.start()]
        .chars()
        .rev()
        .take(12)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>()
        .to_ascii_lowercase();
    !prefix.ends_with("not ")
        && !prefix.ends_with("no ")
        && !prefix.ends_with("isn't ")
        && !prefix.ends_with("is not ")
        && !evidence.contains(&phrase)
}

fn is_search_grounding_attachment_from_text(text: &str) -> bool {
    text.trim_start().starts_with("Local Web Search Context")
        || text.trim_start().starts_with("Active Web Page Context")
}

pub(super) fn grounding_internal_context_leak(response: &str) -> bool {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN
        .get_or_init(|| {
            Regex::new(r"(?i)\blocal[_ -]web[_ -]search(?:_(?:\d+|summary))?\.md\b|\blocal\s+web\s+search\s+context\b")
                .expect("valid internal grounding label regex")
        })
        .is_match(response)
}

pub(super) fn clean_grounding_labels(
    mut response: InferenceResponse,
    attachments: &[ChatAttachment],
) -> InferenceResponse {
    let mut internal_heading_was_supplied = false;
    for attachment in attachments {
        let Some(text) = attachment.text.as_deref() else {
            continue;
        };
        if !is_search_grounding_attachment(attachment, text)
            || !internal_grounding_attachment_name(&attachment.name)
        {
            continue;
        }
        response.text = replace_ascii_case_insensitive(
            response.text,
            &attachment.name,
            super::grounding_contract::PUBLIC_SOURCE_LABEL,
        );
        internal_heading_was_supplied |=
            text.trim_start().lines().next().is_some_and(|heading| {
                heading.trim_end_matches('\r') == "Local Web Search Context"
            });
    }
    if internal_heading_was_supplied {
        response.text = replace_ascii_case_insensitive(
            response.text,
            "Local Web Search Context",
            super::grounding_contract::PUBLIC_SOURCE_LABEL,
        );
    }
    response
}

fn internal_grounding_attachment_name(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    normalized == "local_web_search.md"
        || normalized == "local_web_search_summary.md"
        || normalized
            .strip_prefix("local_web_search_")
            .and_then(|suffix| suffix.strip_suffix(".md"))
            .is_some_and(|suffix| {
                !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
            })
}

fn replace_ascii_case_insensitive(value: String, needle: &str, replacement: &str) -> String {
    debug_assert!(needle.is_ascii());
    let lowercase = value.to_ascii_lowercase();
    let lowercase_needle = needle.to_ascii_lowercase();
    if !lowercase.contains(&lowercase_needle) {
        return value;
    }

    let mut sanitized = String::with_capacity(value.len());
    let mut cursor = 0;
    while let Some(offset) = lowercase[cursor..].find(&lowercase_needle) {
        let start = cursor + offset;
        sanitized.push_str(&value[cursor..start]);
        sanitized.push_str(replacement);
        cursor = start + needle.len();
    }
    sanitized.push_str(&value[cursor..]);
    sanitized
}

pub(super) fn grounded_objective_coverage_missing(
    response: &str,
    attachments: &[ChatAttachment],
) -> bool {
    let queries = grounding_query_tokens(attachments);
    if queries.len() < 2 {
        return false;
    }

    let distinguishing = distinguishing_query_tokens(&queries);
    let response_tokens = exact_tokens(response);
    distinguishing.iter().any(|tokens| {
        !tokens
            .iter()
            .any(|token| response_tokens.contains(token.as_str()))
    })
}

pub(super) fn grounded_requested_field_coverage_missing(
    response: &str,
    objective: &str,
    attachments: &[ChatAttachment],
) -> bool {
    if !requests_release_date_comparison(objective) {
        return false;
    }
    let queries = grounding_query_tokens(attachments);
    if queries.len() < 2 {
        return false;
    }
    let subjects = distinguishing_query_tokens(&queries);
    if subjects.iter().all(|subject| {
        subject_specific_sections(response, subject, &subjects)
            .iter()
            .any(|section| release_date_or_explicit_deficit(section))
    }) {
        return false;
    }

    !response.lines().any(|line| {
        let line_tokens = exact_tokens(line);
        let names_every_subject = subjects
            .iter()
            .all(|subject| subject.iter().any(|token| line_tokens.contains(token)));
        let required_dates = if line_tokens.contains("both") {
            1
        } else {
            subjects.len()
        };
        names_every_subject && release_date_pattern().find_iter(line).count() >= required_dates
    })
}

fn requests_release_date_comparison(value: &str) -> bool {
    let normalized = value.to_lowercase();
    let concepts: &[(&[&str], &[&str])] = &[
        (
            &["compare", "comparison", "contrast"],
            &[
                "release date",
                "release dates",
                "date of release",
                "dates of release",
            ],
        ),
        (
            &["vergleich", "gegenüberstell"],
            &["veröffentlichungsdatum", "veröffentlichungsdaten"],
        ),
        (
            &["compara", "comparación", "contrasta"],
            &["fecha de lanzamiento", "fechas de lanzamiento"],
        ),
        (
            &["compar", "contraste"],
            &["date de sortie", "dates de sortie"],
        ),
        (&["bandingkan", "perbandingan"], &["tanggal rilis"]),
        (&["比較"], &["リリース日", "発売日"]),
        (
            &["compare", "comparação", "contraste"],
            &["data de lançamento", "datas de lançamento"],
        ),
        (
            &["сравн", "сопостав"],
            &[
                "дата релиза",
                "даты релиза",
                "датам релиза",
                "датами релиза",
                "датах релиза",
                "дата выпуска",
                "даты выпуска",
                "датам выпуска",
                "датами выпуска",
                "датах выпуска",
            ],
        ),
        (
            &["порівн", "зістав"],
            &[
                "дата релізу",
                "дати релізу",
                "датам релізу",
                "датами релізу",
                "датах релізу",
                "дата випуску",
                "дати випуску",
                "датам випуску",
                "датами випуску",
                "датах випуску",
            ],
        ),
        (&["so sánh", "đối chiếu"], &["ngày phát hành"]),
        (&["比较", "对比"], &["发布日期", "发布日"]),
        (&["比較", "對比"], &["發佈日期", "發行日期", "發佈日"]),
    ];
    concepts.iter().any(|(comparisons, fields)| {
        contains_any(&normalized, comparisons) && contains_any(&normalized, fields)
    })
}

fn grounding_query_tokens(attachments: &[ChatAttachment]) -> Vec<HashSet<String>> {
    attachments
        .iter()
        .filter_map(|attachment| attachment.text.as_deref())
        .filter_map(|text| {
            text.lines()
                .find_map(|line| line.trim().strip_prefix("Query:"))
                .map(str::trim)
                .filter(|query| !query.is_empty())
        })
        .map(significant_query_tokens)
        .filter(|tokens| !tokens.is_empty())
        .collect()
}

fn distinguishing_query_tokens(queries: &[HashSet<String>]) -> Vec<HashSet<String>> {
    let mut token_frequency = HashMap::<String, usize>::new();
    for tokens in queries {
        for token in tokens {
            *token_frequency.entry(token.clone()).or_default() += 1;
        }
    }
    queries
        .iter()
        .map(|tokens| {
            let distinguishing = tokens
                .iter()
                .filter(|token| token_frequency.get(*token).copied().unwrap_or_default() == 1)
                .cloned()
                .collect::<HashSet<_>>();
            if distinguishing.is_empty() {
                tokens.clone()
            } else {
                distinguishing
            }
        })
        .collect()
}

fn subject_specific_sections(
    response: &str,
    subject: &HashSet<String>,
    all_subjects: &[HashSet<String>],
) -> Vec<String> {
    let lines = response.lines().collect::<Vec<_>>();
    let mut sections = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let line_tokens = exact_tokens(line);
        if !subject.iter().any(|token| line_tokens.contains(token))
            || all_subjects
                .iter()
                .filter(|candidate| candidate.iter().any(|token| line_tokens.contains(token)))
                .count()
                != 1
        {
            continue;
        }
        let mut section = String::new();
        for candidate in lines.iter().skip(index).take(10) {
            let candidate_tokens = exact_tokens(candidate);
            if !section.is_empty()
                && all_subjects.iter().any(|other| {
                    other != subject && other.iter().any(|token| candidate_tokens.contains(token))
                })
            {
                break;
            }
            if !section.is_empty() {
                section.push('\n');
            }
            section.push_str(candidate);
            if section.chars().count() >= 1_200 {
                break;
            }
        }
        sections.push(section);
    }
    sections
}

fn release_date_or_explicit_deficit(value: &str) -> bool {
    release_date_pattern().is_match(value) || explicit_release_date_deficit(value)
}

fn release_date_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(
        r"(?ix)
        \b(?:19|20)\d{2}-\d{1,2}-\d{1,2}\b
        |\b\d{1,2}[/-]\d{1,2}[/-](?:19|20)?\d{2}\b
        |\b(?:jan(?:uary)?|feb(?:ruary)?|mar(?:ch)?|apr(?:il)?|may|jun(?:e)?|jul(?:y)?|aug(?:ust)?|sep(?:tember)?|oct(?:ober)?|nov(?:ember)?|dec(?:ember)?)\s+\d{1,2}(?:st|nd|rd|th)?[,]?\s+(?:19|20)\d{2}\b
        |\b\d{1,2}(?:st|nd|rd|th)?\s+(?:jan(?:uary)?|feb(?:ruary)?|mar(?:ch)?|apr(?:il)?|may|jun(?:e)?|jul(?:y)?|aug(?:ust)?|sep(?:tember)?|oct(?:ober)?|nov(?:ember)?|dec(?:ember)?)[,]?\s+(?:19|20)\d{2}\b
        |\b\d{1,2}(?:\.|º)?\s+(?:(?:de|del)\s+)?(?:
            januar|january|enero|janvier|janeiro|januari|января|січня
            |februar|febrero|février|fevereiro|februari|февраля|лютого
            |märz|marzo|mars|março|maret|марта|березня
            |april|abril|avril|апреля|квітня
            |mai|mayo|may|maio|mei|мая|травня
            |juni|junio|juin|junho|июня|червня
            |juli|julio|juillet|julho|июля|липня
            |august|agosto|août|agustus|августа|серпня
            |september|septiembre|septembre|setembro|сентября|вересня
            |oktober|octubre|octobre|outubro|октября|жовтня
            |november|noviembre|novembre|novembro|ноября|листопада
            |dezember|diciembre|décembre|dezembro|desember|декабря|грудня
        )(?:\s+(?:de|del))?\s+(?:19|20)\d{2}(?:\s*(?:г\.?|года|року))?\b
        |\b(?:ngày\s+)?\d{1,2}\s+tháng\s+\d{1,2}\s+năm\s+(?:19|20)\d{2}\b
        |(?:19|20)\d{2}年\d{1,2}月\d{1,2}日"
    ).expect("valid release date regex"))
}

fn explicit_release_date_deficit(value: &str) -> bool {
    let normalized = value.to_lowercase();
    contains_any(&normalized, RELEASE_DATE_FIELDS)
        && contains_any(&normalized, RELEASE_DATE_DEFICITS)
        && contains_any(&normalized, RELEASE_DATE_EVIDENCE)
}

fn contains_any(value: &str, candidates: &[&str]) -> bool {
    candidates.iter().any(|candidate| value.contains(candidate))
}

fn significant_query_tokens(query: &str) -> HashSet<String> {
    const SEARCH_INTENT_TERMS: &[&str] = &[
        "and", "at", "current", "find", "for", "from", "latest", "official", "on", "public",
        "recent", "release", "releases", "research", "search", "source", "sources", "stable",
        "the", "to", "version", "website", "with",
    ];
    exact_tokens(query)
        .into_iter()
        .filter(|token| token.chars().count() >= 2)
        .filter(|token| !SEARCH_INTENT_TERMS.contains(&token.as_str()))
        .collect()
}

fn exact_tokens(value: &str) -> HashSet<String> {
    value
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_lowercase)
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OutputMockeryViolation {
    PlaceholderCurrency,
    PlaceholderTerm,
    EmptyEvidence,
}

impl OutputMockeryViolation {
    pub(super) fn code(self) -> &'static str {
        match self {
            Self::PlaceholderCurrency => "placeholder_currency",
            Self::PlaceholderTerm => "placeholder_term",
            Self::EmptyEvidence => "empty_evidence",
        }
    }

    pub(super) fn honest_deficit(self) -> &'static str {
        match self {
            Self::PlaceholderCurrency => {
                "A required numeric value is unresolved, so I could not validate this output. No action was performed from the invalid result."
            }
            Self::PlaceholderTerm => {
                "The document output still contains an unresolved template value, so it was not accepted. Nothing was written from the invalid result."
            }
            Self::EmptyEvidence => {
                "Required evidence or result values are missing, so I could not validate the output. No action was performed from the invalid result."
            }
        }
    }
}

pub(super) fn output_mockery_violation(text: &str) -> Option<OutputMockeryViolation> {
    if parsed_tool_envelope(text) {
        return None;
    }
    let prose = prose_for_integrity_scan(text);
    static PLACEHOLDER_CURRENCY: OnceLock<Regex> = OnceLock::new();
    static PLACEHOLDER_TERM: OnceLock<Regex> = OnceLock::new();
    static EMPTY_EVIDENCE: OnceLock<Regex> = OnceLock::new();
    let placeholder_currency = PLACEHOLDER_CURRENCY.get_or_init(|| {
        Regex::new(r"(?i)\$(?:x|y|z|a|n)(?:,(?:x|y|z|a|n){3}|[,\d]+(?:x|y|z|a|n))\b")
            .expect("valid placeholder currency regex")
    });
    if placeholder_currency.is_match(&prose) {
        return Some(OutputMockeryViolation::PlaceholderCurrency);
    }
    let placeholder_term = PLACEHOLDER_TERM.get_or_init(|| {
        Regex::new(
            r"(?ix)(?:\b(?:lorem\s+ipsum|fake_price)\b|\{\{\s*placeholder\s*\}\}|\b(?:amount|citation|date|name|price|total|value)\s*:\s*placeholder\b)",
        )
        .expect("valid placeholder term regex")
    });
    if placeholder_term.is_match(&prose) {
        return Some(OutputMockeryViolation::PlaceholderTerm);
    }
    let empty_evidence = EMPTY_EVIDENCE.get_or_init(|| {
        Regex::new(r"(?i)\b(?:citations?|data|evidence|results?|sources?|values?)\s*:\s*(?:\[\s*\]|\{\s*\})")
            .expect("valid empty evidence regex")
    });
    empty_evidence
        .is_match(&prose)
        .then_some(OutputMockeryViolation::EmptyEvidence)
}

pub(super) fn validate_zero_mockery_with_retry<T>(
    candidate: T,
    text_of: fn(&T) -> &str,
    mut record_violation: impl FnMut(&OutputMockeryViolation, usize, &T) -> Result<(), InferenceError>,
    mut retry: impl FnMut(&OutputMockeryViolation) -> Result<T, InferenceError>,
    mut honest_deficit: impl FnMut(&OutputMockeryViolation, T) -> T,
) -> Result<(T, bool), InferenceError> {
    let Some(violation) = output_mockery_violation(text_of(&candidate)) else {
        return Ok((candidate, false));
    };
    record_violation(&violation, 1, &candidate)?;
    let repaired = retry(&violation)?;
    if let Some(retry_violation) = output_mockery_violation(text_of(&repaired)) {
        record_violation(&retry_violation, 2, &repaired)?;
        return Ok((honest_deficit(&retry_violation, repaired), true));
    }
    Ok((repaired, true))
}

pub(super) fn zero_mockery_repair_system_prompt(system_prompt: &str) -> String {
    format!(
        "{}\n\n{}\nGenerate one fresh replacement answer to the latest user turn. Use only verified values already present in trusted context. Preserve requested Markdown, code, schemas, and literal notation. If a required value is unavailable, name that exact deficit and state whether any action occurred. Return only the clean replacement answer.",
        system_prompt.trim(),
        ZERO_MOCKERY_RETRY_WARNING
    )
}

fn parsed_tool_envelope(text: &str) -> bool {
    let Ok(Value::Object(object)) = serde_json::from_str::<Value>(text.trim()) else {
        return false;
    };
    object.contains_key("operation")
        || object.contains_key("tool")
        || object.contains_key("action")
        || object.contains_key("tool_call")
}

fn prose_for_integrity_scan(text: &str) -> String {
    let mut prose = String::with_capacity(text.len());
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
            fenced = !fenced;
            prose.push('\n');
            continue;
        }
        if fenced {
            prose.push('\n');
            continue;
        }
        let without_tasks = line.replace("- [ ]", "").replace("* [ ]", "");
        let mut inline_code = false;
        for character in without_tasks.chars() {
            if character == '`' {
                inline_code = !inline_code;
                prose.push(' ');
            } else if inline_code {
                prose.push(' ');
            } else {
                prose.push(character);
            }
        }
        prose.push('\n');
    }
    prose
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inference_response(text: &str) -> InferenceResponse {
        InferenceResponse {
            provider_id: "local".to_string(),
            provider: "Local".to_string(),
            model_id: "gemma-4-e4b".to_string(),
            text: text.to_string(),
            response_id: None,
            finish_reason: Some("stop".to_string()),
            latency_ms: 0,
            local_usage: None,
        }
    }

    fn search_attachment(name: &str, query: &str) -> ChatAttachment {
        ChatAttachment {
            name: name.to_string(),
            mime_type: "text/markdown".to_string(),
            byte_count: query.len(),
            data_base64: None,
            text: Some(format!("Local Web Search Context\nQuery: {query}\n\n{{}}")),
            approved_file_receipt: None,
        }
    }

    fn release_attachments() -> Vec<ChatAttachment> {
        vec![
            search_attachment(
                "local_web_search.md",
                "latest stable Rust release date official website",
            ),
            search_attachment(
                "local_web_search_2.md",
                "latest stable Node.js release date official website",
            ),
        ]
    }

    #[test]
    fn technical_markdown_and_code_are_not_fabrication_signatures() {
        for text in [
            "- [ ] Implement route binding",
            "Use [] and {} as empty collection literals, then bind $x.",
            "Discuss the word placeholder in this parser design.",
            "`Evidence: []` is a literal example.",
            "```json\n{\"value\": \"placeholder\"}\n```",
            r#"{"operation":"file_write","content":"- [ ] Implement route binding\nUse {} and placeholder."}"#,
        ] {
            assert_eq!(output_mockery_violation(text), None, "{text}");
        }
    }

    #[test]
    fn unresolved_claim_values_remain_blocked_with_accurate_categories() {
        assert_eq!(
            output_mockery_violation("Flight total: $X,XXX"),
            Some(OutputMockeryViolation::PlaceholderCurrency)
        );
        assert_eq!(
            output_mockery_violation("Price: placeholder"),
            Some(OutputMockeryViolation::PlaceholderTerm)
        );
        assert_eq!(
            output_mockery_violation("Evidence: []"),
            Some(OutputMockeryViolation::EmptyEvidence)
        );
        assert!(!OutputMockeryViolation::EmptyEvidence
            .honest_deficit()
            .contains("Live data"));
    }

    #[test]
    fn multi_search_answers_must_represent_each_distinct_query() {
        let attachments = release_attachments();

        assert!(grounded_objective_coverage_missing(
            "Node.js 24 is the current LTS release.",
            &attachments,
        ));
        assert!(!grounded_objective_coverage_missing(
            "Rust 1.97.1 and Node.js 24 are the current releases reported by their official sources.",
            &attachments,
        ));
    }

    #[test]
    fn grounded_search_rejects_provider_capability_refusals() {
        let attachments = vec![search_attachment(
            "local_web_search.md",
            "the Red Sox are playing today July 27 2026",
        )];
        let response = inference_response(
            "I do not have real-time access to external sports schedules or live internet browsing capabilities.",
        );

        assert_eq!(
            chat_response_retry_reason(
                &response,
                "Check online to see if the Red Sox are playing today, July 27, 2026",
                false,
                true,
                &attachments,
            ),
            Some("grounded_capability_refusal"),
        );
        assert!(is_grounded_repair_reason("grounded_capability_refusal"));
    }

    #[test]
    fn official_source_requests_reject_mixed_first_and_third_party_citations() {
        let objective = "Check the official Rust release notes for the latest stable version.";
        let official_only = concat!(
            "Official sources: https://blog.rust-lang.org/2026/07/16/Rust-1.97.1/ ",
            "and https://doc.rust-lang.org/stable/releases.html"
        );
        let mixed = format!("{official_only} and https://releases.rs/docs/1.97.1/");

        assert!(!grounded_nonofficial_citation_for_official_request(
            official_only,
            objective,
        ));
        assert!(grounded_nonofficial_citation_for_official_request(
            &mixed, objective,
        ));
        assert!(!grounded_nonofficial_citation_for_official_request(
            "Sources: https://example.com/release and https://mirror.example/release",
            objective,
        ));
    }

    #[test]
    fn grounded_answers_cannot_invent_material_severity() {
        let neutral = ChatAttachment {
            name: "local_web_search.md".to_string(),
            mime_type: "text/markdown".to_string(),
            byte_count: 0,
            data_base64: None,
            text: Some(
                "Local Web Search Context\nQuery: Rust 1.97.1\n\n{\"pages\":[{\"visibleText\":\"Fix miscompilation in LLVM optimization\"}]}"
                    .to_string(),
            ),
            approved_file_receipt: None,
        };
        let source_characterized = ChatAttachment {
            text: Some(
                "Local Web Search Context\nQuery: security bulletin\n\n{\"pages\":[{\"visibleText\":\"This is a critical security update.\"}]}"
                    .to_string(),
            ),
            ..neutral.clone()
        };

        assert!(grounded_unsupported_material_intensifier(
            "Install the critical LLVM fix.",
            std::slice::from_ref(&neutral),
        ));
        let sanitized = neutralize_unsupported_material_intensifiers(
            "Install the critical LLVM fix; it prevents a catastrophic compiler result.",
            std::slice::from_ref(&neutral),
        )
        .expect("unsupported severity should be neutralized");
        assert_eq!(
            sanitized,
            "Install the documented LLVM fix; it prevents a documented compiler result."
        );
        assert!(!grounded_unsupported_material_intensifier(
            &sanitized,
            std::slice::from_ref(&neutral),
        ));
        assert!(!grounded_unsupported_material_intensifier(
            "The fix is not critical according to the supplied notes.",
            std::slice::from_ref(&neutral),
        ));
        assert!(neutralize_unsupported_material_intensifiers(
            "The fix is not critical according to the supplied notes.",
            std::slice::from_ref(&neutral),
        )
        .is_none());
        assert!(!grounded_unsupported_material_intensifier(
            "Install the critical security update.",
            std::slice::from_ref(&source_characterized),
        ));
        assert!(neutralize_unsupported_material_intensifiers(
            "Install the critical security update.",
            std::slice::from_ref(&source_characterized),
        )
        .is_none());
    }

    #[test]
    fn release_date_comparison_requires_a_date_or_explicit_deficit_per_subject() {
        let objective =
            "Compare the latest stable Rust and Node.js releases, including their release dates.";
        let attachments = release_attachments();

        assert!(grounded_requested_field_coverage_missing(
            "## Rust\nRust 1.97.1 was released July 16, 2026.\n\n## Node.js\nNode.js v24.18.0 is LTS and v26.5.0 is Current.",
            objective,
            &attachments,
        ));
        assert!(!grounded_requested_field_coverage_missing(
            "## Rust\nRust 1.97.1 was released July 16, 2026.\n\n## Node.js\nNode.js v26.5.0 was released July 22, 2026; v24.18.0 is the LTS line.",
            objective,
            &attachments,
        ));
        assert!(!grounded_requested_field_coverage_missing(
            "## Rust\nRust 1.97.1 was released July 16, 2026.\n\n## Node.js\nThe verified official Node.js source did not provide a release date.",
            objective,
            &attachments,
        ));
    }

    #[test]
    fn localized_and_alternate_release_date_contracts_preserve_exact_deficits() {
        let attachments = release_attachments();
        for (objective, exact_deficit) in [
            (
                "Contrasta las versiones estables de Rust y Node.js y sus fechas de lanzamiento.",
                "## Rust\nRust 1.97.1: 2026-07-16.\n\n## Node.js\nLa fuente oficial de Node.js no proporcionó una fecha de lanzamiento.",
            ),
            (
                "Rust と Node.js の安定版のリリース日を比較してください。",
                "## Rust\nRust 1.97.1: 2026-07-16.\n\n## Node.js\nNode.js の公式情報源にはリリース日が記載されていない。",
            ),
            (
                "Contrast the stable Rust and Node.js versions by their dates of release.",
                "## Rust\nRust 1.97.1: 2026-07-16.\n\n## Node.js\nThe official Node.js source did not provide a date of release.",
            ),
        ] {
            assert!(grounded_requested_field_coverage_missing(
                "## Rust\nRust 1.97.1: 2026-07-16.\n\n## Node.js\nNode.js v24 is LTS.",
                objective,
                &attachments,
            ));
            assert!(!grounded_requested_field_coverage_missing(
                exact_deficit,
                objective,
                &attachments,
            ));
        }
    }

    #[test]
    fn localized_natural_release_dates_satisfy_requested_field_coverage() {
        let attachments = release_attachments();
        for (objective, rust_date, node_date) in [
            (
                "Vergleiche Rust und Node.js anhand ihrer Veröffentlichungsdaten.",
                "16. Juli 2026",
                "22. Juli 2026",
            ),
            (
                "Compara Rust y Node.js por sus fechas de lanzamiento.",
                "16 de julio de 2026",
                "22 de julio de 2026",
            ),
            (
                "Compare Rust et Node.js par leurs dates de sortie.",
                "16 juillet 2026",
                "22 juillet 2026",
            ),
            (
                "Bandingkan Rust dan Node.js berdasarkan tanggal rilis.",
                "16 Juli 2026",
                "22 Juli 2026",
            ),
            (
                "Rust と Node.js のリリース日を比較してください。",
                "2026年7月16日",
                "2026年7月22日",
            ),
            (
                "Compare Rust e Node.js pelas datas de lançamento.",
                "16 de julho de 2026",
                "22 de julho de 2026",
            ),
            (
                "Сравните Rust и Node.js по датам релиза.",
                "16 июля 2026 года",
                "22 июля 2026 года",
            ),
            (
                "Порівняйте Rust і Node.js за датами релізу.",
                "16 липня 2026 року",
                "22 липня 2026 року",
            ),
            (
                "So sánh Rust và Node.js theo ngày phát hành.",
                "ngày 16 tháng 7 năm 2026",
                "ngày 22 tháng 7 năm 2026",
            ),
            (
                "比较 Rust 和 Node.js 的发布日期。",
                "2026年7月16日",
                "2026年7月22日",
            ),
        ] {
            assert!(
                requests_release_date_comparison(objective),
                "localized objective did not activate release-date validation: {objective}"
            );
            let response = format!(
                "## Rust\nRust 1.97.1: {rust_date}.\n\n## Node.js\nNode.js 26.5.0: {node_date}."
            );
            assert!(
                !grounded_requested_field_coverage_missing(&response, objective, &attachments,),
                "localized natural dates were rejected: {response}"
            );
        }
    }

    #[test]
    fn internal_search_context_filenames_are_grounding_violations() {
        assert!(grounding_internal_context_leak(
            "I analyzed local_web_search.md and local_web_search_2.md."
        ));
        assert!(grounding_internal_context_leak(
            "The Local Web Search Context supplied these facts."
        ));
        assert!(!grounding_internal_context_leak(
            "I compared the verified Rust and Node.js sources."
        ));
        assert!(is_grounded_repair_reason("grounded_internal_context_leak"));
        assert!(is_grounded_repair_reason(
            "grounded_requested_field_coverage"
        ));
    }

    #[test]
    fn active_internal_grounding_labels_are_neutralized_without_touching_evidence() {
        let source = "https://thenewstack.io/moonshot-fable5-release/";
        let mut attachment = search_attachment("local_web_search.md", "Moonshot Fable 5 release");
        attachment.text = Some(format!(
            "Local Web Search Context\nQuery: Moonshot Fable 5 release\n\n{}",
            serde_json::json!({"accessedAtUtc":"2026-07-23T14:12:13.456Z","pages":[{"url":source}]})
        ));
        let response = inference_response(&format!(
            "## Local Web Search Context\nFable 5 was announced in July 2026 ([local_web_search.md]({source}))."
        ));

        let sanitized = clean_grounding_labels(response, std::slice::from_ref(&attachment));

        assert!(!grounding_internal_context_leak(&sanitized.text));
        assert!(sanitized
            .text
            .contains("Fable 5 was announced in July 2026"));
        assert!(sanitized.text.contains(source));
        assert_eq!(sanitized.text.matches(source).count(), 1);
        assert_eq!(
            chat_response_retry_reason(
                &sanitized,
                "Research the Moonshot Fable 5 release and cite the source.",
                false,
                true,
                &[attachment],
            ),
            None
        );
    }

    #[test]
    fn sanitizer_is_fail_closed_for_unbound_or_mutated_internal_labels() {
        let response = inference_response("The Local Web Search Context supplied these facts.");
        let unchanged = clean_grounding_labels(response.clone(), &[]);
        assert_eq!(unchanged.text, response.text);
        assert!(grounding_internal_context_leak(&unchanged.text));

        let attachment = search_attachment("local_web_search.md", "official source");
        let mutated = clean_grounding_labels(
            inference_response("I relied on local-web-search.md."),
            &[attachment],
        );
        assert!(grounding_internal_context_leak(&mutated.text));
    }

    #[test]
    fn sanitizing_labels_does_not_weaken_grounded_objective_coverage() {
        let mut attachments = release_attachments();
        attachments[1].text = Some(format!(
            "Local Web Search Context\nQuery: latest stable Node.js release date official website\n\n{}",
            serde_json::json!({"accessedAtUtc":"2026-07-23T14:12:13.456Z","pages":[{"url":"https://nodejs.org/"}]})
        ));
        let response = clean_grounding_labels(
            inference_response(
                "Local Web Search Context: Node.js 24 is the current LTS release from https://nodejs.org/.",
            ),
            &attachments,
        );

        assert_eq!(
            chat_response_retry_reason(
                &response,
                "Compare the latest stable Rust and Node.js releases.",
                false,
                true,
                &attachments,
            ),
            Some("grounded_objective_coverage")
        );
    }
}
