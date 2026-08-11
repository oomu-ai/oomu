use regex::Regex;
use std::collections::HashSet;

pub(super) fn localized_explicit_search_query(query: &str) -> Option<String> {
    let normalized = query.trim();
    let patterns = [
        r"(?iu)^(?:bitte\s+)?suche\s+(?:im|bei|mit)\s+(?:google|duckduckgo|internet|web)\s+(?:nach\s+)?(.+)$",
        r"(?iu)^(?:por\s+favor[,:]?\s+)?busca\s+(?:en|con)\s+(?:google|duckduckgo|internet|la\s+web)\s+(?:por\s+)?(.+)$",
        r"(?iu)^(?:s['’]il\s+vous\s+pla[iî]t[,:]?\s+)?(?:recherche|recherchez|cherche|cherchez)\s+(?:sur|avec)\s+(?:google|duckduckgo|internet|le\s+web)\s+(.+)$",
        r"(?iu)^(?:tolong\s+)?cari\s+(?:di|dengan)\s+(?:google|duckduckgo|internet|web)\s+(.+)$",
        r"(?iu)^(?:google|duckduckgo|インターネット|ウェブ)(?:で|を使って)(.+?)(?:を)?検索(?:して|してください)?$",
        r"(?iu)^(?:por\s+favor[,:]?\s+)?pesquis(?:e|ar)\s+(?:no|na|com)\s+(?:google|duckduckgo|internet|web)\s+(?:por\s+)?(.+)$",
        r"(?iu)^(?:пожалуйста[,:]?\s+)?(?:найди|найдите|поищи|поищите)\s+(?:в|через)\s+(?:google|duckduckgo|интернете|сети)\s+(.+)$",
        r"(?iu)^(?:будь\s+ласка[,:]?\s+)?(?:знайди|знайдіть|пошукай|пошукайте)\s+(?:в|через)\s+(?:google|duckduckgo|інтернеті|мережі)\s+(.+)$",
        r"(?iu)^(?:vui\s+lòng\s+)?tìm\s+kiếm\s+(?:trên|bằng)\s+(?:google|duckduckgo|internet|web)\s+(?:về\s+)?(.+)$",
        r"(?iu)^(?:请)?(?:在|用)\s*(?:google|duckduckgo|互联网|网络)(?:上)?\s*搜索\s*(.+)$",
        r"(?iu)^(?:請)?(?:在|用)\s*(?:google|duckduckgo|網際網路|網路)(?:上)?\s*搜尋\s*(.+)$",
    ];
    patterns.iter().find_map(|pattern| {
        let regex = Regex::new(pattern).expect("localized search directive regex is valid");
        let topic = regex.captures(normalized)?.get(1)?.as_str();
        let topic = clean_localized_topic(topic);
        (!topic.is_empty()
            && !search_topic_is_weak_or_deictic(&topic)
            && !localized_private_search_target(&topic))
        .then_some(topic)
    })
}

pub(super) fn freshness_search_requested(query: &str) -> bool {
    let normalized = query.trim().to_lowercase();
    if normalized.is_empty() {
        return false;
    }
    let english = [
        "latest",
        "most recent",
        "breaking",
        "today",
        "tonight",
        "tomorrow",
        "this week",
        "this month",
        "this year",
        "newest",
        "right now",
        "up to date",
        "currently",
        "current news",
        "current weather",
        "current status",
        "current score",
        "current schedule",
        "weather",
        "standings",
        "fixtures",
    ];
    if english.iter().any(|marker| normalized.contains(marker)) {
        return true;
    }
    [
        r"(?iu)\b(?:heute|aktuell|neueste[nrsm]?)\b",
        r"(?iu)\b(?:hoy|actual(?:es)?|m[aá]s\s+reciente)\b",
        r"(?iu)\b(?:aujourd['’]hui|actuel(?:le|les|s)?|plus\s+r[eé]cent)\b",
        r"(?iu)\b(?:hari\s+ini|terbaru|saat\s+ini)\b",
        r"(?:今日|現在|最新)",
        r"(?iu)\b(?:hoje|atual|mais\s+recente)\b",
        r"(?iu)\b(?:сегодня|текущ\p{L}*|последн\p{L}*)\b",
        r"(?iu)\b(?:сьогодні|поточн\p{L}*|останн\p{L}*)\b",
        r"(?iu)\b(?:h[oô]m\s+nay|hiện\s+tại|mới\s+nhất)\b",
        r"(?:今天|目前|最新)",
    ]
    .iter()
    .any(|pattern| {
        Regex::new(pattern)
            .expect("freshness regex is valid")
            .is_match(&normalized)
    })
}

pub(super) fn localized_private_search_target(query: &str) -> bool {
    let normalized = query.to_lowercase();
    [
        ("my", "calendar"),
        ("our", "calendar"),
        ("mein", "kalender"),
        ("mi", "calendario"),
        ("mon", "calendrier"),
        ("kalender saya", ""),
        ("私のカレンダー", ""),
        ("meu", "calendário"),
        ("мой", "календар"),
        ("мій", "календар"),
        ("lịch của tôi", ""),
        ("我", "日历"),
        ("我", "行事曆"),
    ]
    .iter()
    .any(|(owner, target)| normalized.contains(owner) && normalized.contains(target))
}

pub(super) fn search_topic_is_weak_or_deictic(topic: &str) -> bool {
    let normalized = topic
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    if normalized.is_empty() || normalized.len() > 4 {
        return normalized.is_empty();
    }
    let weak_words = [
        "a",
        "about",
        "an",
        "and",
        "answer",
        "are",
        "can",
        "check",
        "confirm",
        "das",
        "danach",
        "did",
        "do",
        "does",
        "eso",
        "for",
        "für",
        "have",
        "i",
        "in",
        "is",
        "isso",
        "it",
        "itu",
        "look",
        "me",
        "of",
        "on",
        "online",
        "por",
        "pour",
        "please",
        "search",
        "that",
        "the",
        "these",
        "this",
        "those",
        "to",
        "up",
        "untuk",
        "verify",
        "was",
        "web",
        "were",
        "what",
        "where",
        "who",
        "you",
        "cela",
        "это",
        "це",
        "それ",
        "でそれ",
        "那个",
        "那個",
        "điều",
        "đó",
    ];
    normalized
        .iter()
        .all(|word| weak_words.contains(&word.as_str()))
}

fn clean_localized_topic(topic: &str) -> String {
    topic
        .trim()
        .trim_matches(|character: char| {
            character.is_whitespace()
                || matches!(
                    character,
                    '"' | '\'' | '`' | '“' | '”' | '‘' | '’' | '。' | '！' | '？' | '!' | '?'
                )
        })
        .trim()
        .to_string()
}

pub(super) fn explicit_external_search_requested(query: &str) -> bool {
    query
        .split(['.', '!', '?', ';', '\n'])
        .map(search_directive_tokens)
        .any(|tokens| {
            explicit_external_search_directive(&tokens)
                || explicit_go_online_research_directive(&tokens)
                || explicit_natural_web_research_directive(&tokens)
                || explicit_direct_web_research_directive(&tokens)
                || explicit_named_public_source_research_directive(&tokens)
                || explicit_coordinated_web_research_directive(&tokens)
        })
}

fn explicit_go_online_research_directive(tokens: &[String]) -> bool {
    matches!(
        tokens,
        [go, online, and, action, rest @ ..]
            if go == "go"
                && online == "online"
                && and == "and"
                && matches!(action.as_str(), "research" | "search" | "find")
                && !rest.is_empty()
                && !research_clause_is_negated(tokens)
    ) || matches!(
        tokens,
        [research, online, rest @ ..]
            if research == "research"
                && online == "online"
                && !rest.is_empty()
                && !research_clause_is_negated(tokens)
    )
}

fn explicit_natural_web_research_directive(tokens: &[String]) -> bool {
    if tokens.is_empty()
        || research_clause_is_negated(tokens)
        || crate::local_app_intent::has_private_app_data_intent(&tokens.join(" "))
    {
        return false;
    }

    let public_surface = |token: &str| {
        matches!(
            token,
            "web" | "internet" | "online" | "google" | "duckduckgo"
        )
    };
    let search_action = |token: &str| {
        matches!(
            token,
            "browse"
                | "check"
                | "confirm"
                | "consult"
                | "explore"
                | "find"
                | "investigate"
                | "look"
                | "research"
                | "search"
                | "see"
                | "verify"
        )
    };

    match tokens.first().map(String::as_str) {
        Some("take" | "have") => {
            let Some(look) = tokens.iter().position(|token| token == "look") else {
                return false;
            };
            let Some(surface) = tokens
                .iter()
                .enumerate()
                .skip(look + 1)
                .find_map(|(index, token)| public_surface(token).then_some(index))
            else {
                return false;
            };
            surface + 1 < tokens.len()
        }
        Some("do" | "run" | "conduct" | "perform") => {
            let Some(surface) = tokens
                .iter()
                .enumerate()
                .skip(1)
                .find_map(|(index, token)| public_surface(token).then_some(index))
            else {
                return false;
            };
            tokens.get(surface + 1).is_some_and(|token| {
                matches!(token.as_str(), "search" | "check" | "lookup" | "research")
            }) && surface + 2 < tokens.len()
        }
        Some("go") => {
            let Some(surface) = tokens
                .iter()
                .enumerate()
                .skip(1)
                .find_map(|(index, token)| public_surface(token).then_some(index))
            else {
                return false;
            };
            tokens
                .iter()
                .enumerate()
                .skip(surface + 1)
                .find(|(_, token)| token.as_str() == "and")
                .and_then(|(and, _)| tokens.get(and + 1))
                .is_some_and(|token| matches!(token.as_str(), "find" | "research" | "search"))
        }
        Some("see") => {
            let Some(surface) = tokens
                .iter()
                .enumerate()
                .skip(1)
                .find_map(|(index, token)| public_surface(token).then_some(index))
            else {
                return false;
            };
            tokens[..surface].iter().any(|token| token == "find") && surface + 1 < tokens.len()
        }
        Some(action) if search_action(action) => {
            tokens.iter().enumerate().skip(2).any(|(surface, token)| {
                let connector = surface.checked_sub(1).and_then(|previous| {
                    let token = tokens.get(previous)?;
                    if matches!(token.as_str(), "the" | "public") {
                        previous.checked_sub(1).and_then(|index| tokens.get(index))
                    } else {
                        Some(token)
                    }
                });
                public_surface(token)
                    && connector.is_some_and(|previous| {
                        matches!(previous.as_str(), "on" | "using" | "across")
                    })
                    && surface + 1 == tokens.len()
            })
        }
        _ => false,
    }
}

pub(super) fn independent_public_research_query_allowed(objective: &str, query: &str) -> bool {
    objective.split(['.', '!', '?', ';', '\n']).any(|clause| {
        let tokens = search_directive_tokens(clause);
        if explicit_direct_web_research_directive(&tokens)
            || explicit_named_public_source_research_directive(&tokens)
        {
            return query_is_public_subset(&tokens[1..], query);
        }
        tokens
            .windows(2)
            .position(|pair| pair[0] == "independently" && pair[1] == "research")
            .is_some_and(|research| {
                explicit_coordinated_web_research_directive(&tokens)
                    && query_is_public_subset(&tokens[research + 2..], query)
            })
    })
}

fn search_directive_tokens(clause: &str) -> Vec<String> {
    super::strip_search_courtesy_prefix(clause)
        .split(|character: char| !character.is_alphanumeric() && character != '\'')
        .filter(|token| !token.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn explicit_external_search_directive(tokens: &[String]) -> bool {
    let Some(first) = tokens.first().map(String::as_str) else {
        return false;
    };
    if matches!(
        first,
        "how" | "what" | "why" | "when" | "where" | "who" | "did" | "does" | "do"
    ) {
        return false;
    }
    match first {
        "search" | "browse" | "check" | "confirm" | "find" | "research" | "see" | "verify" => {
            !(first == "check"
                && tokens.get(1).is_some_and(|token| token == "google")
                && tokens.get(2).is_some_and(|token| token == "calendar"))
                && external_search_surface_after(tokens, 1).is_some()
        }
        "look" => {
            external_search_surface_after(tokens, 1).is_some()
                || tokens
                    .iter()
                    .position(|token| token == "up")
                    .is_some_and(|up| has_look_up_external_locator(tokens, up + 1))
        }
        "use" => external_search_surface_after(tokens, 1).is_some_and(|surface| {
            tokens
                .iter()
                .position(|token| token == "to")
                .filter(|to| *to > surface)
                .is_some_and(|to| {
                    let operative = &tokens[to.saturating_add(1)..];
                    operative.iter().enumerate().any(|(index, token)| {
                        matches!(
                            token.as_str(),
                            "search"
                                | "browse"
                                | "check"
                                | "confirm"
                                | "find"
                                | "research"
                                | "see"
                                | "verify"
                        ) || (token == "look"
                            && operative.get(index + 1).is_some_and(|next| next == "up"))
                    })
                })
        }),
        _ => false,
    }
}

pub(super) fn objective_bound_refined_query_allowed(objective: &str, query: &str) -> bool {
    if !explicit_external_search_requested(objective)
        || query.trim().is_empty()
        || objective
            .split(|character: char| !character.is_alphanumeric())
            .filter(|token| !token.is_empty())
            .map(str::to_ascii_lowercase)
            .eq(query
                .split(|character: char| !character.is_alphanumeric())
                .filter(|token| !token.is_empty())
                .map(str::to_ascii_lowercase))
        || query.contains(['/', '\\', '@'])
        || crate::local_app_intent::has_private_app_data_intent(objective)
        || crate::local_app_intent::has_private_app_data_intent(query)
        || localized_private_search_target(objective)
        || localized_private_search_target(query)
    {
        return false;
    }
    let objective_tokens = significant_public_tokens(objective)
        .into_iter()
        .collect::<HashSet<_>>();
    let query_tokens = significant_public_tokens(query);
    let safe_refinements = HashSet::from([
        "date".to_string(),
        "dates".to_string(),
        "latest".to_string(),
        "notes".to_string(),
        "official".to_string(),
        "public".to_string(),
        "release".to_string(),
        "releases".to_string(),
        "stable".to_string(),
        "website".to_string(),
        "websites".to_string(),
    ]);
    !query_tokens.is_empty()
        && query_tokens.iter().all(|token| {
            objective_tokens.contains(token)
                || safe_refinements.contains(token)
                || is_version_refinement_token(token)
        })
}

fn is_version_refinement_token(token: &str) -> bool {
    Regex::new(r"(?i)^v?\d+(?:\.\d+){1,3}(?:[-+][a-z0-9.-]+)?$")
        .expect("static version refinement regex")
        .is_match(token)
}

pub(super) fn separate_release_queries(utterance: &str, primary: String) -> Vec<String> {
    let separate = Regex::new(r"(?i)\bsearch\s+each\s+separately\b")
        .expect("static separate search regex")
        .is_match(utterance);
    if !separate {
        return vec![primary];
    }
    let subjects = Regex::new(
        r"(?i)\b(?:latest\s+)?stable\s+releases?\s+of\s+(.+?)\s+and\s+(.+?)\s+from\s+(?:their\s+)?official\s+websites?\b",
    )
    .expect("static release subjects regex");
    let Some(captures) = subjects.captures(utterance) else {
        return vec![primary];
    };
    let clean = |value: &str| {
        value
            .trim()
            .trim_matches(['"', '\'', '`', ',', '.'])
            .to_string()
    };
    let left = captures.get(1).map(|value| clean(value.as_str()));
    let right = captures.get(2).map(|value| clean(value.as_str()));
    match (left, right) {
        (Some(left), Some(right)) if !left.is_empty() && !right.is_empty() => {
            let requests_dates = Regex::new(r"(?i)\brelease\s+dates?\b")
                .expect("static release date regex")
                .is_match(utterance);
            if requests_dates {
                vec![
                    format!("latest stable {left} release date official website"),
                    format!("latest stable {right} release date official website"),
                ]
            } else {
                vec![
                    format!("latest stable release of {left} official website"),
                    format!("latest stable release of {right} official website"),
                ]
            }
        }
        _ => vec![primary],
    }
}

fn significant_public_tokens(value: &str) -> Vec<String> {
    value
        .split(|character: char| !character.is_alphanumeric() && character != '.')
        .filter(|token| token.len() >= 2)
        .map(str::to_ascii_lowercase)
        .filter(|token| {
            !matches!(
                token.as_str(),
                "about"
                    | "and"
                    | "can"
                    | "compare"
                    | "each"
                    | "find"
                    | "for"
                    | "from"
                    | "go"
                    | "online"
                    | "research"
                    | "search"
                    | "separately"
                    | "the"
                    | "their"
                    | "what"
                    | "you"
            )
        })
        .collect()
}

fn explicit_direct_web_research_directive(tokens: &[String]) -> bool {
    if !tokens.first().is_some_and(|token| token == "research") {
        return false;
    }
    let scope = &tokens[1..];
    let Some(web) = scope.iter().position(|token| token == "web") else {
        return false;
    };
    if !scope
        .get(web + 1)
        .is_some_and(|token| matches!(token.as_str(), "source" | "sources"))
        || !scope[..web]
            .iter()
            .any(|token| matches!(token.as_str(), "authoritative" | "official" | "primary"))
    {
        return false;
    }
    web_research_scope_is_safe(tokens, scope, web)
}

fn explicit_named_public_source_research_directive(tokens: &[String]) -> bool {
    if !tokens.first().is_some_and(|token| token == "research")
        || research_clause_is_negated(tokens)
        || crate::local_app_intent::has_private_app_data_intent(&tokens.join(" "))
    {
        return false;
    }
    let Some(source) = tokens
        .iter()
        .position(|token| matches!(token.as_str(), "source" | "sources"))
    else {
        return false;
    };
    if source <= 1
        || !tokens[1..source]
            .iter()
            .any(|token| matches!(token.as_str(), "authoritative" | "official" | "primary"))
    {
        return false;
    }
    let topic_start = match tokens.get(source + 1).map(String::as_str) {
        Some("about" | "for" | "on" | "regarding") => source + 2,
        Some("relevant") if tokens.get(source + 2).is_some_and(|token| token == "to") => source + 3,
        _ => return false,
    };
    if topic_start >= tokens.len() {
        return false;
    }
    !targets_local_research_material(&tokens[topic_start..])
}

fn research_clause_is_negated(tokens: &[String]) -> bool {
    tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "not"
                | "no"
                | "never"
                | "without"
                | "don't"
                | "dont"
                | "didn't"
                | "didnt"
                | "isn't"
                | "isnt"
                | "wasn't"
                | "wasnt"
        )
    })
}

fn targets_local_research_material(topic: &[String]) -> bool {
    topic.windows(2).any(|pair| {
        matches!(pair[0].as_str(), "my" | "our" | "this" | "the" | "local")
            && matches!(
                pair[1].as_str(),
                "document"
                    | "documents"
                    | "calendar"
                    | "calendars"
                    | "contact"
                    | "contacts"
                    | "email"
                    | "emails"
                    | "file"
                    | "files"
                    | "folder"
                    | "folders"
                    | "inbox"
                    | "mail"
                    | "messages"
                    | "notes"
                    | "photos"
                    | "reminders"
                    | "repository"
                    | "schedule"
                    | "workspace"
            )
    })
}

fn explicit_coordinated_web_research_directive(tokens: &[String]) -> bool {
    let Some(research) = tokens
        .windows(2)
        .position(|pair| pair[0] == "independently" && pair[1] == "research")
    else {
        return false;
    };
    let direct = research == 0;
    let coordinated = research >= 2
        && tokens.get(research - 1).is_some_and(|token| token == "and")
        && coordinated_imperative_prefix(&tokens[..research - 1]);
    if !direct && !coordinated {
        return false;
    }
    let scope = &tokens[research + 2..];
    let Some(web) = scope.iter().position(|token| token == "web") else {
        return false;
    };
    if !scope
        .get(web + 1)
        .is_some_and(|token| matches!(token.as_str(), "source" | "sources"))
    {
        return false;
    }
    web_research_scope_is_safe(tokens, scope, web)
}

fn web_research_scope_is_safe(tokens: &[String], scope: &[String], web: usize) -> bool {
    let authorization_span_len = tokens.len() - scope.len() + web + 2;
    let authorization_span = &tokens[..authorization_span_len];
    let unsafe_scope = authorization_span.iter().any(|token| {
        matches!(
            token.as_str(),
            "not"
                | "no"
                | "never"
                | "without"
                | "don't"
                | "dont"
                | "didn't"
                | "didnt"
                | "isn't"
                | "isnt"
                | "wasn't"
                | "wasnt"
                | "how"
                | "why"
                | "whether"
                | "if"
        )
    }) || scope[..web + 2].iter().any(|token| {
        matches!(
            token.as_str(),
            "local"
                | "repository"
                | "document"
                | "documents"
                | "file"
                | "files"
                | "folder"
                | "folders"
        )
    });
    !unsafe_scope
}

fn coordinated_imperative_prefix(tokens: &[String]) -> bool {
    tokens.first().is_some_and(|token| {
        matches!(
            token.as_str(),
            "analyze"
                | "assess"
                | "check"
                | "compare"
                | "identify"
                | "inspect"
                | "read"
                | "reconcile"
                | "review"
                | "verify"
        )
    })
}

fn external_search_surface_after(tokens: &[String], start: usize) -> Option<usize> {
    let mut index = start;
    if tokens
        .get(index)
        .is_some_and(|token| matches!(token.as_str(), "on" | "using"))
    {
        index += 1;
    }
    if tokens.get(index).is_some_and(|token| token == "the") {
        index += 1;
    }
    if tokens.get(index).is_some_and(|token| token == "public") {
        index += 1;
    }
    tokens.get(index).and_then(|token| {
        matches!(
            token.as_str(),
            "web" | "internet" | "online" | "google" | "duckduckgo"
        )
        .then_some(index)
    })
}

fn has_look_up_external_locator(tokens: &[String], start: usize) -> bool {
    tokens.iter().skip(start).any(|token| token == "online")
        || tokens.iter().enumerate().skip(start).any(|(index, token)| {
            matches!(token.as_str(), "on" | "using")
                && external_search_surface_after(tokens, index).is_some()
        })
}

pub(super) fn query_is_public_subset(authorized_scope: &[String], query: &str) -> bool {
    if query.trim().is_empty()
        || query.contains(['/', '\\', '@'])
        || crate::local_app_intent::has_private_app_data_intent(query)
    {
        return false;
    }
    let scope = authorized_scope.iter().collect::<HashSet<_>>();
    let query_tokens = query
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_ascii_lowercase)
        .filter(|token| {
            !matches!(
                token.as_str(),
                "a" | "an"
                    | "and"
                    | "current"
                    | "for"
                    | "official"
                    | "or"
                    | "primary"
                    | "source"
                    | "sources"
                    | "the"
                    | "web"
            )
        })
        .collect::<Vec<_>>();
    !query_tokens.is_empty()
        && !query_tokens
            .iter()
            .any(|token| matches!(token.as_str(), "my" | "our"))
        && query_tokens.iter().all(|token| scope.contains(token))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_subset_excludes_private_or_unapproved_material() {
        let scope = ["official", "fuel", "freight", "conditions"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert!(query_is_public_subset(&scope, "fuel freight conditions"));
        assert!(!query_is_public_subset(&scope, "my calendar conflicts"));
        assert!(!query_is_public_subset(&scope, "supplier secret 48291"));
        assert!(!query_is_public_subset(&scope, "/tmp/private.txt"));
    }
}
