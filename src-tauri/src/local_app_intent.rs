use std::collections::HashSet;

const RELEVANCE_THRESHOLD: f64 = 0.75;
const BM25_K1: f64 = 1.2;
const BM25_B: f64 = 0.75;

const TECHNICAL_EXCLUSIONS: &[&str] = &[
    "algorithm",
    "algorithms",
    "asynchronous",
    "callback",
    "callbacks",
    "compilation",
    "compiler",
    "computational",
    "cpu",
    "cvxpy",
    "descent",
    "gpu",
    "ipc",
    "kernel",
    "loop",
    "loops",
    "mesh",
    "network",
    "networks",
    "optimization",
    "packet",
    "packets",
    "process",
    "processes",
    "programming",
    "queue",
    "queues",
    "semidefinite",
    "solver",
    "thread",
    "threads",
];

const CALENDAR_TRIGGERS: &[&str] = &[
    "appointment",
    "appointments",
    "agenda",
    "calendar",
    "calendars",
    "event",
    "events",
    "meeting",
    "meetings",
    "schedule",
    "scheduled",
];
const MAIL_TRIGGERS: &[&str] = &[
    "email", "emails", "inbox", "mail", "message", "messages", "reply", "replies", "unread",
];
const REMINDERS_TRIGGERS: &[&str] = &["reminder", "reminders", "task", "tasks", "todo", "todos"];
const NOTES_TRIGGERS: &[&str] = &["note", "notes"];
const CONTACTS_TRIGGERS: &[&str] = &["contact", "contacts", "phonebook", "rolodex"];
const PHOTOS_TRIGGERS: &[&str] = &["album", "camera", "image", "photo", "picture", "library"];
const MUSIC_TRIGGERS: &[&str] = &["album", "library", "music", "song", "track"];

const CALENDAR_ANCHORS: &[&str] = &[
    "check my appointments",
    "read my calendar",
    "what meetings do I have tomorrow",
    "show my scheduled events",
    "what is my schedule tomorrow",
    "what is on my calendar today",
    "do I have any meetings",
];
const MAIL_ANCHORS: &[&str] = &[
    "read my unread emails",
    "check my inbox",
    "draft an email reply",
    "show my latest messages",
    "check my unread email",
    "review my recent mail",
    "open a Mail draft",
];
const REMINDERS_ANCHORS: &[&str] = &[
    "show my to-do list",
    "add a reminder",
    "check my pending tasks",
    "what are my outstanding todos",
    "check my reminders for anything open",
    "do I have outstanding reminders",
];
const NOTES_ANCHORS: &[&str] = &[
    "read my notes",
    "show my recent notes",
    "find a note in Apple Notes",
    "check my Apple Notes",
    "create a note in Apple Notes",
    "write a new note",
];
const CONTACTS_ANCHORS: &[&str] = &[
    "show my contacts",
    "check my contacts",
    "look up a contact",
    "search my address book",
    "find a person in contacts",
];
const PHOTOS_ANCHORS: &[&str] = &[
    "show my newest photo",
    "what is the newest photo in my albums",
    "check my photo library",
    "find my latest image",
    "show my recent photos",
];
const MUSIC_ANCHORS: &[&str] = &[
    "which song did I add most recently to my music library",
    "show my recently added Apple Music songs",
    "find an album in my music library",
    "check my Apple Music library",
];

const PRIVATE_APP_KINDS: &[&str] = &[
    "calendar",
    "mail",
    "reminders",
    "notes",
    "contacts",
    "photos",
    "music",
];

#[derive(Clone, Copy)]
struct IntentProfile {
    triggers: &'static [&'static str],
    anchors: &'static [&'static str],
}

pub(crate) fn has_local_app_intent(prompt: &str, app_kind: &str) -> bool {
    let Some(profile) = intent_profile(app_kind) else {
        return false;
    };
    let normalized = normalize_phrases(prompt);
    let normalized_app_kind = app_kind.trim().to_ascii_lowercase();
    if matches!(normalized_app_kind.as_str(), "mail" | "email")
        && explicitly_targets_apple_messages(&normalized)
    {
        return false;
    }
    if matches!(normalized_app_kind.as_str(), "reminder" | "reminders")
        && has_oomu_task_context(&normalized)
        && !has_explicit_reminders_evidence(&normalized)
    {
        return false;
    }
    let tokens = tokenize(&normalized);

    if !contains_any(&tokens, profile.triggers) {
        return false;
    }
    // `scheduled` is common product/research vocabulary (for example,
    // "scheduled/background agent capabilities"). It is not, by itself, a
    // request to inspect Calendar. Only let this otherwise-ambiguous trigger
    // enter Calendar scoring when the utterance also names a Calendar object
    // or binds the scheduling language to the user's private Calendar scope.
    if normalized_app_kind == "calendar"
        && !has_explicit_calendar_object(&normalized)
        && !has_bound_private_calendar_scope(&normalized)
    {
        return false;
    }
    if matches!(
        app_kind.trim().to_ascii_lowercase().as_str(),
        "photo" | "photos" | "music" | "media"
    ) {
        return has_personal_library_read_shape(&normalized, app_kind);
    }
    if has_deterministic_private_app_shape(&normalized, &tokens, app_kind) {
        return true;
    }
    if contains_any(&tokens, TECHNICAL_EXCLUSIONS) {
        return false;
    }

    bm25_anchor_relevance(&tokens, profile.anchors) >= RELEVANCE_THRESHOLD
}

pub(crate) fn is_focused_local_app_shortcut_request(prompt: &str, app_kind: &str) -> bool {
    let normalized = normalize_phrases(prompt);
    if normalized.trim().is_empty() {
        return false;
    }

    let requested_formats = [
        ".csv", ".docx", ".html", ".json", ".md", ".pdf", ".pptx", ".rtf", ".txt", ".xls", ".xlsx",
        ".xml",
    ]
    .iter()
    .filter(|extension| normalized.contains(**extension))
    .count();
    let has_file_action = [
        "read ",
        "inspect ",
        "review ",
        "analyze ",
        "analyse ",
        "reconcile ",
        "compare ",
        "generate ",
        "produce ",
        "deliver ",
        "write ",
        "save ",
        "export ",
    ]
    .iter()
    .any(|marker| normalized.contains(marker));
    let has_file_object = [
        " file",
        " folder",
        " director",
        " workbook",
        " spreadsheet",
        " presentation",
        " document",
        ".csv",
        ".docx",
        ".json",
        ".md",
        ".pdf",
        ".pptx",
        ".txt",
        ".xlsx",
    ]
    .iter()
    .any(|marker| normalized.contains(marker));
    let has_web_action = ["research ", "search ", "browse ", "verify ", "lookup "]
        .iter()
        .any(|marker| normalized.contains(marker));
    let has_web_object = [
        " web",
        "internet",
        "online",
        "official source",
        "primary source",
        " url",
    ]
    .iter()
    .any(|marker| normalized.contains(marker));
    let has_structural_execution = has_recurring_automation_intent(&normalized)
        || (["run ", "execute "]
            .iter()
            .any(|marker| normalized.contains(marker))
            && [
                "command", "script", "binary", "program", "terminal", "shell", "workflow",
                "taskflow", "test", "build", "compile", "npm", "npx", "pnpm", "yarn", "cargo",
                "python", "node", "bash", "zsh", "make",
            ]
            .iter()
            .any(|marker| normalized.contains(marker)));
    let has_connector_work = [
        "post ", "publish ", "share ", "upload ", "notify ", "message ",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
        && ["slack", "teams", "discord", "notion", "jira", "linear"]
            .iter()
            .any(|marker| normalized.contains(marker));
    let informational = is_informational_local_app_question(&normalized);

    let app_kind = debug_app_kind_label(app_kind);
    let other_local_app_action = [
        (
            "calendar",
            [
                "create ",
                "add ",
                "schedule ",
                "book ",
                "move ",
                "update ",
                "cancel ",
                "delete ",
            ]
            .as_slice(),
            ["calendar", "event", "meeting", "appointment"].as_slice(),
        ),
        (
            "mail",
            ["open ", "create ", "compose ", "draft ", "reply ", "send "].as_slice(),
            ["mail", "email", "inbox", "draft"].as_slice(),
        ),
        (
            "reminders",
            ["create ", "add ", "set ", "complete ", "delete "].as_slice(),
            ["reminder", "todo"].as_slice(),
        ),
        (
            "notes",
            ["create ", "add ", "save ", "write ", "delete "].as_slice(),
            ["note", "apple notes"].as_slice(),
        ),
    ]
    .iter()
    .any(|(candidate_kind, actions, objects)| {
        *candidate_kind != app_kind
            && actions.iter().any(|marker| normalized.contains(marker))
            && objects.iter().any(|marker| normalized.contains(marker))
    });

    !contains_local_path_reference(prompt)
        && requested_formats <= 1
        && !(has_file_action && has_file_object)
        && !(has_web_action && has_web_object)
        && !has_structural_execution
        && !has_connector_work
        && !informational
        && !has_non_negated_private_app_mutation(&normalized)
        && !other_local_app_action
}

fn has_recurring_automation_intent(normalized: &str) -> bool {
    const NAMED_CADENCES: &str = concat!(
        "hourly daily nightly weekly monthly quarterly yearly annually ",
        "periodically recurring recurrent repeatedly",
    );
    const CADENCE_UNITS: &str = concat!(
        "minute minutes hour hours day days night nights week weeks month months ",
        "quarter quarters year years weekday weekdays weekend weekends morning mornings ",
        "afternoon afternoons evening evenings monday mondays tuesday tuesdays ",
        "wednesday wednesdays thursday thursdays friday fridays saturday saturdays sunday sundays",
    );
    const EXECUTABLE_WORK: &str = concat!(
        "check read review scan search summarize summarise report ",
        "run execute send create update",
    );
    let table_contains = |table: &str, word: &str| {
        table
            .split_ascii_whitespace()
            .any(|candidate| candidate == word)
    };
    let words = normalized
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    let has_named_cadence = words
        .iter()
        .any(|word| table_contains(NAMED_CADENCES, word));
    let has_relative_cadence = words.iter().enumerate().any(|(index, word)| {
        matches!(*word, "every" | "each")
            && words
                .iter()
                .skip(index + 1)
                .take(3)
                .any(|candidate| table_contains(CADENCE_UNITS, candidate))
    });
    let has_executable_work = words
        .iter()
        .any(|word| table_contains(EXECUTABLE_WORK, word));
    (has_named_cadence || has_relative_cadence) && has_executable_work
}

pub(crate) fn is_informational_local_app_question(prompt: &str) -> bool {
    let normalized = normalize_phrases(prompt);
    [
        "how do i ",
        "how can i ",
        "how should i ",
        "how does ",
        "explain ",
        "tell me about ",
        "tell me how ",
        "help me understand ",
    ]
    .iter()
    .any(|prefix| normalized.starts_with(prefix))
        || (normalized.starts_with("how many ")
            && ![" my ", " our ", " do i ", " did i ", " have i ", " for me "]
                .iter()
                .any(|marker| format!(" {normalized} ").contains(marker)))
}

fn has_non_negated_private_app_mutation(normalized: &str) -> bool {
    normalized
        .split(|character| matches!(character, '.' | '!' | '?' | ';'))
        .any(|clause| {
            let words = clause
                .split(|character: char| !character.is_alphanumeric() && character != '\'')
                .filter(|word| !word.is_empty())
                .collect::<Vec<_>>();
            words.iter().enumerate().any(|(index, word)| {
                let mutation = matches!(
                    *word,
                    "archive"
                        | "archives"
                        | "archived"
                        | "archiving"
                        | "delete"
                        | "deletes"
                        | "deleted"
                        | "deleting"
                        | "trash"
                        | "move"
                        | "moves"
                        | "moved"
                        | "moving"
                        | "forward"
                        | "forwards"
                        | "forwarded"
                        | "forwarding"
                        | "flag"
                        | "flags"
                        | "flagged"
                        | "flagging"
                        | "star"
                        | "stars"
                        | "starred"
                        | "starring"
                        | "label"
                        | "tag"
                        | "mark"
                        | "send"
                        | "sends"
                        | "sending"
                        | "sent"
                );
                if !mutation {
                    return false;
                }
                let prefix = words[..index].join(" ");
                let suffix = words.get(index + 1).copied().unwrap_or_default();
                !["do not", "don't", "dont", "never", "without", "not to"]
                    .iter()
                    .any(|negation| prefix.ends_with(negation))
                    && suffix != "nothing"
            })
        })
}

fn contains_local_path_reference(prompt: &str) -> bool {
    const ABSOLUTE_ROOTS: &[&str] = &[
        "Applications",
        "Library",
        "System",
        "Users",
        "Volumes",
        "home",
        "opt",
        "private",
        "tmp",
        "usr",
        "var",
    ];
    prompt.split_whitespace().any(|token| {
        let candidate = token.trim_matches(|character: char| {
            matches!(
                character,
                '`' | '"'
                    | '\''
                    | '<'
                    | '>'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | ','
                    | ';'
                    | ':'
                    | '!'
                    | '?'
            )
        });
        if candidate.starts_with("file://")
            || candidate.starts_with("~/")
            || candidate.starts_with("./")
            || candidate.starts_with("../")
        {
            return true;
        }
        if let Some(absolute) = candidate.strip_prefix('/') {
            let root = absolute.split('/').next().unwrap_or_default();
            return absolute.contains('/') || ABSOLUTE_ROOTS.contains(&root);
        }
        if candidate.contains("://") || !candidate.contains('/') {
            return false;
        }
        let mut components = candidate.split('/').filter(|part| !part.is_empty());
        let first = components.next().unwrap_or_default();
        let second = components.next().unwrap_or_default();
        !second.is_empty()
            && (first.contains(['.', '_', '-'])
                || second.contains(['.', '_', '-'])
                || components.next().is_some())
    })
}

fn explicitly_targets_apple_messages(normalized: &str) -> bool {
    [
        "message app",
        "messages app",
        "message application",
        "messages application",
        "apple message",
        "apple messages",
        "imessage",
        "imessages",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

fn has_explicit_reminders_evidence(normalized: &str) -> bool {
    normalized
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .any(|token| matches!(token, "reminder" | "reminders" | "todo" | "todos"))
        || normalized.contains("remind me")
}

fn has_oomu_task_context(normalized: &str) -> bool {
    let words = normalized
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect::<HashSet<_>>();
    let has_task = words.contains("task") || words.contains("tasks");
    let has_oomu_surface = words.contains("project")
        || words.contains("projects")
        || words.contains("workflow")
        || words.contains("workflows")
        || words.contains("oomu");
    let has_task_ui = [
        "task screen",
        "tasks screen",
        "task tab",
        "tasks tab",
        "task view",
        "tasks view",
        "task panel",
        "tasks panel",
        "task page",
        "tasks page",
    ]
    .iter()
    .any(|marker| normalized.contains(marker));

    has_task && (has_oomu_surface || has_task_ui)
}

fn has_deterministic_private_app_shape(
    normalized: &str,
    tokens: &[String],
    app_kind: &str,
) -> bool {
    let padded = format!(" {normalized} ");
    let personal_scope = [" my ", " our ", " do i ", " did i ", " have i ", " for me "]
        .iter()
        .any(|marker| padded.contains(marker));
    let explicit_app_scope = explicit_app_scope(normalized, app_kind);
    let action_evidence = tokens
        .iter()
        .any(|token| matches!(token.as_str(), "check" | "add" | "draft"))
        || [
            " what ",
            " what's ",
            " which ",
            " who ",
            " when ",
            " are there ",
            " how many ",
            " do i have ",
        ]
        .iter()
        .any(|marker| padded.contains(marker));

    action_evidence && (personal_scope || explicit_app_scope)
}

fn explicit_app_scope(normalized: &str, app_kind: &str) -> bool {
    let markers: &[&str] = match app_kind.trim().to_ascii_lowercase().as_str() {
        "calendar" => &["calendar app", "apple calendar", "macos calendar"],
        "mail" | "email" => &["mail app", "apple mail", "gmail inbox", "outlook inbox"],
        "reminder" | "reminders" => &["reminders app", "apple reminders"],
        "note" | "notes" => &["notes app", "apple notes"],
        "contact" | "contacts" => &["contacts app", "apple contacts"],
        "photo" | "photos" => &["photos app", "apple photos", "photo library"],
        "music" | "media" => &["music app", "apple music library", "music library"],
        _ => &[],
    };
    markers.iter().any(|marker| normalized.contains(marker))
}

fn has_explicit_calendar_object(normalized: &str) -> bool {
    normalized
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .any(|token| {
            matches!(
                token,
                "agenda"
                    | "appointment"
                    | "appointments"
                    | "calendar"
                    | "calendars"
                    | "event"
                    | "events"
                    | "meeting"
                    | "meetings"
            )
        })
}

fn has_bound_private_calendar_scope(normalized: &str) -> bool {
    if explicit_app_scope(normalized, "calendar") {
        return true;
    }

    let words = normalized
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    let calendar_indexes = words
        .iter()
        .enumerate()
        .filter_map(|(index, token)| CALENDAR_TRIGGERS.contains(token).then_some(index))
        .collect::<Vec<_>>();

    calendar_indexes.into_iter().any(|calendar_index| {
        let possessive_start = calendar_index.saturating_sub(4);
        let possessive = words[possessive_start..calendar_index]
            .iter()
            .any(|token| matches!(*token, "my" | "our"));
        if possessive {
            return true;
        }

        let nearby_start = calendar_index.saturating_sub(8);
        let nearby_end = (calendar_index + 9).min(words.len());
        let nearby = &words[nearby_start..nearby_end];
        nearby.windows(2).any(|pair| {
            matches!(
                pair,
                ["do", "i"] | ["did", "i"] | ["have", "i"] | ["for", "me"]
            )
        })
    })
}

/// Returns the private app-data surface explicitly targeted by the current
/// prompt. This is intentionally separate from web/current-facts detection:
/// requests for a user's calendar, mail, reminders, notes, contacts, photos,
/// or music library must never be repaired or substituted with a web search.
pub(crate) fn private_app_data_kind(prompt: &str) -> Option<&'static str> {
    let normalized = normalize_phrases(prompt);
    let padded = format!(" {normalized} ");
    let personal_scope = [" my ", " our ", " do i ", " did i ", " have i ", " for me "]
        .iter()
        .any(|marker| padded.contains(marker));
    let messages_data_scope = [
        " unread ",
        " recent ",
        " latest ",
        " conversation ",
        " conversations ",
        " chat ",
        " chats ",
        " thread ",
        " threads ",
    ]
    .iter()
    .any(|marker| padded.contains(marker));
    if explicitly_targets_apple_messages(&normalized) && (personal_scope || messages_data_scope) {
        return Some("messages");
    }
    let mentions_external_surface = [
        " web ",
        " internet ",
        " online ",
        " google ",
        " duckduckgo ",
    ]
    .iter()
    .any(|marker| padded.contains(marker));
    let has_private_scope = personal_scope
        || normalized.starts_with("my ")
        || normalized.contains(" do i ")
        || normalized.contains(" did i ")
        || normalized.contains(" have i ")
        || normalized.contains(" for me")
        || [
            "calendar app",
            "google calendar",
            "apple calendar",
            "mail app",
            "gmail inbox",
            "reminders app",
            "notes app",
            "contacts app",
            "photos app",
            "music app",
            "apple music library",
        ]
        .iter()
        .any(|marker| normalized.contains(marker));
    if mentions_external_surface && !has_private_scope {
        return None;
    }
    PRIVATE_APP_KINDS.iter().copied().find(|app_kind| {
        if !has_local_app_intent(prompt, app_kind) {
            return false;
        }
        if *app_kind != "calendar" || has_bound_private_calendar_scope(&normalized) {
            return true;
        }
        normalized
            .split(|character: char| !character.is_alphanumeric())
            .filter(|token| !token.is_empty())
            .any(|token| {
                matches!(
                    token,
                    "calendar" | "agenda" | "appointment" | "meeting" | "event"
                )
            })
    })
}

pub(crate) fn has_private_app_data_intent(prompt: &str) -> bool {
    private_app_data_kind(prompt).is_some()
}

fn has_personal_library_read_shape(normalized: &str, app_kind: &str) -> bool {
    if [
        "how do i ",
        "how can i ",
        "how should i ",
        "how does ",
        "how do ",
        "explain ",
        "show me how ",
        "tell me about ",
        "tell me how ",
    ]
    .iter()
    .any(|prefix| normalized.starts_with(prefix))
    {
        return false;
    }
    let words = normalized
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect::<HashSet<_>>();
    let has_app_specific_scope = match app_kind.trim().to_ascii_lowercase().as_str() {
        "photo" | "photos" => {
            ["photo", "photos", "picture", "pictures", "image", "images"]
                .iter()
                .any(|marker| words.contains(marker))
                || ["camera roll", "photos app", "apple photos", "icloud photos"]
                    .iter()
                    .any(|marker| normalized.contains(marker))
        }
        "music" | "media" => {
            [
                "music",
                "song",
                "songs",
                "track",
                "tracks",
                "playlist",
                "playlists",
            ]
            .iter()
            .any(|marker| words.contains(marker))
                || ["music app", "apple music", "media library"]
                    .iter()
                    .any(|marker| normalized.contains(marker))
        }
        _ => false,
    };
    if !has_app_specific_scope {
        return false;
    }
    let directive = [
        "check ",
        "find ",
        "list ",
        "lookup ",
        "read ",
        "review ",
        "scan ",
        "search ",
        "show ",
        "summarize ",
        "summarise ",
    ]
    .iter()
    .any(|marker| normalized.starts_with(marker) || normalized.contains(&format!(" {marker}")));
    let question_or_recency = [
        "what ",
        "what's ",
        "which ",
        "who ",
        "when ",
        "how many ",
        "newest",
        "latest",
        "most recent",
        "last added",
    ]
    .iter()
    .any(|marker| normalized.starts_with(marker) || normalized.contains(&format!(" {marker}")));
    let personal = normalized.starts_with("my ")
        || normalized.contains(" my ")
        || normalized.contains(" do i ")
        || normalized.contains(" did i ")
        || normalized.contains(" have i ")
        || normalized.contains(" for me");
    let protected_location = ["in photos", "from photos", "in music", "from music"]
        .iter()
        .any(|marker| normalized.contains(marker));
    let explicit_app_scope = ["photos app", "music app", "apple music library"]
        .iter()
        .any(|marker| normalized.contains(marker));
    (directive && (personal || protected_location || explicit_app_scope))
        || (question_or_recency && (personal || protected_location))
}

#[tauri::command]
pub(crate) fn triage_local_app_intent(prompt: String, app_kind: String) -> bool {
    let approved = has_local_app_intent(&prompt, &app_kind)
        && is_focused_local_app_shortcut_request(&prompt, &app_kind);
    if crate::debug_trace_enabled() {
        eprintln!(
            "OOMU_LOCAL_APP_TRIAGE app_kind={} approved={} prompt_chars={}",
            debug_app_kind_label(&app_kind),
            approved,
            prompt.chars().count()
        );
    }
    approved
}

fn debug_app_kind_label(app_kind: &str) -> &'static str {
    match app_kind.trim().to_ascii_lowercase().as_str() {
        "calendar" => "calendar",
        "mail" | "email" => "mail",
        "reminder" | "reminders" => "reminders",
        "note" | "notes" => "notes",
        "contact" | "contacts" => "contacts",
        "photo" | "photos" => "photos",
        "music" | "media" => "music",
        _ => "unknown",
    }
}

fn intent_profile(app_kind: &str) -> Option<IntentProfile> {
    match app_kind.trim().to_ascii_lowercase().as_str() {
        "calendar" => Some(IntentProfile {
            triggers: CALENDAR_TRIGGERS,
            anchors: CALENDAR_ANCHORS,
        }),
        "mail" | "email" => Some(IntentProfile {
            triggers: MAIL_TRIGGERS,
            anchors: MAIL_ANCHORS,
        }),
        "reminder" | "reminders" => Some(IntentProfile {
            triggers: REMINDERS_TRIGGERS,
            anchors: REMINDERS_ANCHORS,
        }),
        "note" | "notes" => Some(IntentProfile {
            triggers: NOTES_TRIGGERS,
            anchors: NOTES_ANCHORS,
        }),
        "contact" | "contacts" => Some(IntentProfile {
            triggers: CONTACTS_TRIGGERS,
            anchors: CONTACTS_ANCHORS,
        }),
        "photo" | "photos" => Some(IntentProfile {
            triggers: PHOTOS_TRIGGERS,
            anchors: PHOTOS_ANCHORS,
        }),
        "music" | "media" => Some(IntentProfile {
            triggers: MUSIC_TRIGGERS,
            anchors: MUSIC_ANCHORS,
        }),
        _ => None,
    }
}

fn normalize_phrases(prompt: &str) -> String {
    prompt
        .to_lowercase()
        .replace("e-mail", "email")
        .replace("to-do", "todo")
        .replace("address book", "contacts")
        .replace("phone book", "phonebook")
        .replace("look up", "lookup")
}

fn tokenize(value: &str) -> Vec<String> {
    value
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(canonical_token)
        .filter(|token| !token.is_empty() && !is_stopword(token))
        .map(str::to_string)
        .collect()
}

fn canonical_token(token: &str) -> &str {
    match token {
        "agenda" | "appointment" | "appointments" | "calendars" | "event" | "events"
        | "meeting" | "meetings" | "schedule" | "scheduled" => "calendar",
        "email" | "emails" | "inbox" | "messages" => "mail",
        "replies" => "reply",
        "reminders" | "task" | "tasks" => "reminder",
        "todos" => "todo",
        "notes" => "note",
        "contacts" | "phonebook" | "rolodex" => "contact",
        "images" | "photos" => "photo",
        "pictures" => "picture",
        "albums" => "album",
        "libraries" => "library",
        "playlists" => "playlist",
        "songs" => "song",
        "checking" | "read" | "reading" | "review" | "reviewing" | "show" | "showing"
        | "lookup" | "search" | "searching" | "find" | "finding" | "list" | "listing" | "scan"
        | "scanning" | "see" => "check",
        "adding" | "create" | "creating" | "make" | "making" | "set" => "add",
        "compose" | "composing" | "prepare" | "preparing" | "write" | "writing" => "draft",
        "latest" => "recent",
        _ => token,
    }
}

fn is_stopword(token: &str) -> bool {
    matches!(
        token,
        "a" | "an"
            | "and"
            | "are"
            | "as"
            | "at"
            | "be"
            | "by"
            | "can"
            | "could"
            | "did"
            | "do"
            | "does"
            | "for"
            | "from"
            | "had"
            | "has"
            | "he"
            | "her"
            | "hers"
            | "him"
            | "his"
            | "i"
            | "if"
            | "in"
            | "into"
            | "is"
            | "it"
            | "its"
            | "may"
            | "me"
            | "might"
            | "my"
            | "of"
            | "on"
            | "or"
            | "our"
            | "ours"
            | "please"
            | "should"
            | "than"
            | "that"
            | "the"
            | "their"
            | "theirs"
            | "them"
            | "then"
            | "there"
            | "these"
            | "they"
            | "this"
            | "those"
            | "to"
            | "us"
            | "was"
            | "we"
            | "were"
            | "when"
            | "where"
            | "which"
            | "who"
            | "why"
            | "will"
            | "with"
            | "would"
            | "you"
            | "your"
            | "yours"
    )
}

fn contains_any(tokens: &[String], candidates: &[&str]) -> bool {
    tokens
        .iter()
        .any(|token| candidates.iter().any(|candidate| token == candidate))
}

fn bm25_anchor_relevance(prompt_tokens: &[String], anchors: &[&str]) -> f64 {
    let query = prompt_tokens
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let documents = anchors
        .iter()
        .map(|anchor| {
            let normalized = normalize_phrases(anchor);
            tokenize(&normalized)
        })
        .collect::<Vec<_>>();
    let average_document_length =
        documents.iter().map(Vec::len).sum::<usize>() as f64 / documents.len().max(1) as f64;

    documents
        .iter()
        .map(|document| {
            let terms = document.iter().map(String::as_str).collect::<HashSet<_>>();
            let mut matched_score = 0.0;
            let mut full_score = 0.0;
            for term in terms {
                let document_frequency = documents
                    .iter()
                    .filter(|candidate| candidate.iter().any(|token| token == term))
                    .count() as f64;
                let document_count = documents.len() as f64;
                let inverse_document_frequency = (1.0
                    + (document_count - document_frequency + 0.5) / (document_frequency + 0.5))
                    .ln();
                let term_frequency = document
                    .iter()
                    .filter(|token| token.as_str() == term)
                    .count() as f64;
                let length_normalization = term_frequency
                    + BM25_K1
                        * (1.0 - BM25_B
                            + BM25_B * document.len() as f64 / average_document_length.max(1.0));
                let term_score = inverse_document_frequency * term_frequency * (BM25_K1 + 1.0)
                    / length_normalization;
                full_score += term_score;
                if query.contains(term) {
                    matched_score += term_score;
                }
            }
            if full_score == 0.0 {
                0.0
            } else {
                matched_score / full_score
            }
        })
        .fold(0.0, f64::max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn rejects_technical_false_positives_for_every_productivity_app() {
        let cases = [
            (
                "schedule asynchronous data packets across a mesh network",
                "calendar",
            ),
            ("model IPC message and reply ordering", "mail"),
            ("inspect the CPU task queue", "reminders"),
            ("write notes about a semidefinite solver", "notes"),
            ("find electrical contacts in the GPU process", "contacts"),
            ("show how an event loop handles callbacks", "calendar"),
            ("review message queues in the kernel", "mail"),
            ("list tasks assigned to each CPU thread", "reminders"),
            ("summarize the release notes for this compilation", "notes"),
            ("show network contact resistance values", "contacts"),
            ("optimize the meeting scheduler algorithm", "calendar"),
            ("draft an IPC reply message", "mail"),
            ("show TODO tasks in the programming project", "reminders"),
            ("find notes in the CVXPY solver output", "notes"),
            ("search contacts in the asynchronous mesh graph", "contacts"),
        ];

        for (prompt, app_kind) in cases {
            assert!(!has_local_app_intent(prompt, app_kind), "{prompt}");
        }
    }

    #[test]
    fn accepts_clear_productivity_requests_for_every_supported_app() {
        let cases = [
            ("What is my schedule tomorrow?", "calendar"),
            ("What is on my calendar today?", "calendar"),
            ("Do I have any meetings?", "calendar"),
            ("Show my scheduled events", "calendar"),
            ("Read my unread emails", "mail"),
            ("Do I have any unread emails?", "mail"),
            ("Check my inbox", "mail"),
            ("Draft an email reply", "mail"),
            ("Open a Mail draft", "mail"),
            ("Show my pending tasks", "reminders"),
            ("Add a reminder", "reminders"),
            ("What are my outstanding todos?", "reminders"),
            ("Read my Notes", "notes"),
            ("Show my recent notes", "notes"),
            ("Write a new note", "notes"),
            ("Show my contacts", "contacts"),
            ("Look up a contact", "contacts"),
            ("Search my address book", "contacts"),
            ("What is the newest photo in my photo albums?", "photos"),
            ("Check my photo library", "photos"),
            (
                "Which song did I add most recently to my music library?",
                "music",
            ),
            ("Show my recently added Apple Music songs", "music"),
        ];

        for (prompt, app_kind) in cases {
            assert!(has_local_app_intent(prompt, app_kind), "{prompt}");
        }
    }

    #[test]
    fn do_i_have_mail_question_is_retained_by_server_triage() {
        let prompt = "Do I have any unread emails?";

        assert!(has_local_app_intent(prompt, "mail"));
        assert!(is_focused_local_app_shortcut_request(prompt, "mail"));
        assert!(triage_local_app_intent(
            prompt.to_string(),
            "mail".to_string()
        ));
        assert_eq!(private_app_data_kind(prompt), Some("mail"));
    }

    #[test]
    fn mail_shortcut_triage_rejects_informational_compound_and_mutating_requests() {
        for prompt in [
            "How many unread emails are normal?",
            "Do I have any unread emails? Then run npm test.",
            "Do I have any unread emails? Then post the count to Slack.",
            "Do I have any unread emails? Then flag them.",
            "Do I have any unread emails? Then star them.",
            "Can you set up an hourly task to check my email for any unread messages. Only run for today until midnight tonight. Once you set it up, run it once to ensure it’s working properly. If it does not work properly, report back here and let me know the outcome.",
        ] {
            assert!(
                !is_focused_local_app_shortcut_request(prompt, "mail"),
                "{prompt}"
            );
        }
        for prompt in [
            "Check my email for anything unread.",
            "Do I have any unread emails? Do not mark them as read.",
            "Do I have any unread emails? Move nothing; just summarize.",
        ] {
            assert!(
                is_focused_local_app_shortcut_request(prompt, "mail"),
                "{prompt}"
            );
        }
    }

    #[test]
    fn compound_cross_surface_requests_never_collapse_into_one_app_shortcut() {
        let prompt = "Read mock_data/supplier_proposals.json, reconcile every amount, research current official web sources, create supplier_decision.xlsx, supplier_decision.pptx, supplier_decision.pdf, and sources.md, then create a tentative event in my OOMU Test calendar and create a Mail draft listing the files.";

        assert!(has_local_app_intent(prompt, "mail"));
        assert!(!is_focused_local_app_shortcut_request(prompt, "mail"));
        assert!(!triage_local_app_intent(
            prompt.to_string(),
            "mail".to_string()
        ));
        assert!(is_focused_local_app_shortcut_request(
            "Open a Mail draft saying I am running late.",
            "mail"
        ));

        let directory_prompt = "Use /Users/example/testing/mock_data to inform yourself about the inputs, then create a Mail draft saying the review is ready.";
        assert!(has_local_app_intent(directory_prompt, "mail"));
        assert!(!is_focused_local_app_shortcut_request(
            directory_prompt,
            "mail"
        ));
    }

    #[test]
    fn accepts_explicit_private_app_requests_despite_natural_language_dilution() {
        let cases = [
            (
                "Please search my calendar and see whether there is anything I need to prepare for before tomorrow afternoon",
                "calendar",
            ),
            (
                "Could you search my email and see if there is anything from Maya that I should answer before lunch",
                "mail",
            ),
            (
                "Search my reminders and see whether I still have anything outstanding for the launch",
                "reminders",
            ),
            (
                "Please search my notes and find anything I saved about the product review last month",
                "notes",
            ),
            (
                "Search my contacts and see if you can find Maya Allan",
                "contacts",
            ),
            (
                "Search my photo library and show me the newest picture I added after the conference",
                "photos",
            ),
            (
                "Search my Apple Music library and list the songs I added most recently for the drive",
                "music",
            ),
        ];

        for (prompt, app_kind) in cases {
            assert!(has_local_app_intent(prompt, app_kind), "{prompt}");
            assert_eq!(private_app_data_kind(prompt), Some(app_kind), "{prompt}");
        }
    }

    #[test]
    fn explicit_private_app_evidence_accepts_technical_record_content() {
        for (prompt, app_kind) in [
            (
                "Search my calendar callback schedule in the asynchronous event loop",
                "calendar",
            ),
            ("Search my mail message queue in the kernel process", "mail"),
            (
                "Search my reminder task queue across CPU threads",
                "reminders",
            ),
            (
                "Search my notes about the compiler optimization algorithm",
                "notes",
            ),
            (
                "Search my contacts in the asynchronous mesh graph",
                "contacts",
            ),
            (
                "Search my photo library for the process diagram screenshot",
                "photos",
            ),
            (
                "Search my music library for the song named Network",
                "music",
            ),
        ] {
            assert!(has_local_app_intent(prompt, app_kind), "{prompt}");
            assert_eq!(private_app_data_kind(prompt), Some(app_kind), "{prompt}");
        }
    }

    #[test]
    fn rejects_ambiguous_single_words_and_unknown_app_kinds() {
        assert!(!has_local_app_intent("schedule", "calendar"));
        assert!(!has_local_app_intent("message", "mail"));
        assert!(!has_local_app_intent("task", "reminders"));
        assert!(!has_local_app_intent("album", "photos"));
        assert!(!has_local_app_intent("Read my calendar", "unknown"));
    }

    #[test]
    fn debug_app_kind_labels_are_canonical_and_never_echo_untrusted_input() {
        assert_eq!(debug_app_kind_label("email"), "mail");
        assert_eq!(debug_app_kind_label(" CONTACTS "), "contacts");
        assert_eq!(debug_app_kind_label("private prompt text\n"), "unknown");
    }

    #[test]
    fn product_tasks_do_not_route_to_reminders_without_explicit_reminders_evidence() {
        for prompt in [
            "What are my tasks in this project?",
            "Show the tasks in my OOMU workflow.",
            "Check the Tasks screen for this project.",
        ] {
            assert!(!has_local_app_intent(prompt, "reminders"), "{prompt}");
            assert_ne!(private_app_data_kind(prompt), Some("reminders"), "{prompt}");
        }

        for prompt in [
            "Show my pending tasks.",
            "What are my tasks in this project in Apple Reminders?",
            "Show my project todos.",
        ] {
            assert!(has_local_app_intent(prompt, "reminders"), "{prompt}");
            assert_eq!(private_app_data_kind(prompt), Some("reminders"), "{prompt}");
        }
    }

    #[test]
    fn explicit_messages_ui_is_never_classified_as_private_mail() {
        for prompt in [
            "Summarize my Messages app UI.",
            "Review Apple Messages for unread mail UI text.",
            "Check iMessage for the active thread.",
        ] {
            assert!(!has_local_app_intent(prompt, "mail"), "{prompt}");
            assert_ne!(private_app_data_kind(prompt), Some("mail"), "{prompt}");
        }
        assert_eq!(
            private_app_data_kind("Search online for my iMessages"),
            Some("messages")
        );
        assert!(has_private_app_data_intent(
            "Search online for my iMessages"
        ));
        assert!(!has_local_app_intent(
            "Search online for Apple Messages API documentation",
            "mail"
        ));
    }

    #[test]
    fn planner_private_read_examples_remain_typed_private_app_requests() {
        for prompt in [
            "Check my calendar and tell me what is planned tomorrow.",
            "Find Maya Allan in my contacts.",
            "Look in my contacts and see if you can find Maya Allan.",
            "Show my newest photo.",
            "Read my unread emails.",
        ] {
            assert!(has_private_app_data_intent(prompt), "{prompt}");
        }
    }

    #[test]
    fn private_app_data_kind_identifies_calendar_without_treating_today_as_web_intent() {
        assert_eq!(
            private_app_data_kind("Check my calendar and let me know what I have going on today"),
            Some("calendar")
        );
        assert!(has_private_app_data_intent(
            "What is the newest photo in my photo albums?"
        ));
        assert!(!has_private_app_data_intent(
            "Search the web for today's baseball schedule"
        ));
    }

    #[test]
    fn scheduled_background_research_is_not_private_calendar_intent() {
        let prompt = "Research current primary or official sources on scheduled/background agent capabilities in OpenClaw and Claude Cowork. Write a sourced comparison to ship_test_04/background_agent_comparison.md in my testing folder. Include URLs, access times, explicit limitations, and a section explaining what this implies for OOMU. Do not claim completion until the file exists and you have read it back.";

        assert!(!has_local_app_intent(prompt, "calendar"));
        assert_eq!(private_app_data_kind(prompt), None);
        assert!(!has_private_app_data_intent(prompt));
    }

    #[test]
    fn genuine_calendar_language_remains_bound_to_calendar() {
        for prompt in [
            "What is scheduled in my calendar tomorrow?",
            "Do I have any scheduled events tomorrow?",
            "Research this topic, then create a review event in my OOMU Test calendar.",
        ] {
            assert!(has_local_app_intent(prompt, "calendar"), "{prompt}");
            assert_eq!(private_app_data_kind(prompt), Some("calendar"), "{prompt}");
        }
    }

    #[test]
    fn protected_library_triage_rejects_explanations_without_a_personal_read() {
        for (prompt, app_kind) in [
            ("How does Photos organize albums?", "photos"),
            ("What is a photo album?", "photos"),
            ("What is Apple Music?", "music"),
            ("Explain how music libraries work.", "music"),
            ("Find the latest Apple Music news.", "music"),
        ] {
            assert!(!has_local_app_intent(prompt, app_kind), "{prompt}");
        }
    }

    #[test]
    fn average_scoring_latency_stays_below_one_millisecond() {
        const ITERATIONS: u32 = 2_000;
        let started = Instant::now();
        for _ in 0..ITERATIONS {
            assert!(has_local_app_intent(
                "What is my schedule tomorrow?",
                "calendar"
            ));
        }
        let average = started.elapsed() / ITERATIONS;
        assert!(
            average.as_micros() < 1_000,
            "average latency was {average:?}"
        );
    }
}
