use super::*;
use chrono::{Datelike, Duration, Local, NaiveDate, Timelike, Weekday};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct MailRecipients {
    to: Vec<String>,
    cc: Vec<String>,
    bcc: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ConnectedServiceBinding {
    manifest_id: &'static str,
    allowed_capabilities: &'static [&'static str],
    label: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum CrossSurfaceRequirement {
    CalendarCreate {
        title: Option<String>,
        calendar_name: Option<String>,
        start_minutes_local: Option<u16>,
        duration_minutes: Option<u16>,
        availability: Option<String>,
        day: Option<String>,
        date: Option<String>,
        location: Option<String>,
        notes: Option<String>,
        authority_conflict: bool,
        instruction: String,
    },
    MailDraft {
        recipients: MailRecipients,
        subject: Option<String>,
        body: Option<String>,
        instruction: String,
    },
    MailSend {
        recipients: MailRecipients,
        subject: Option<String>,
        body: Option<String>,
        instruction: String,
    },
    ConnectedWork {
        connector_name: Option<String>,
        capability: Option<String>,
        arguments: Option<Value>,
        semantic_tokens: Vec<String>,
        service_binding: Option<ConnectedServiceBinding>,
        instruction: String,
    },
    ChannelConfigure {
        platform: String,
        is_active: bool,
        owner_id: Option<String>,
        instruction: String,
    },
}

impl CrossSurfaceRequirement {
    pub(super) fn label(&self) -> String {
        match self {
            Self::CalendarCreate {
                title: Some(title), ..
            } => {
                format!("Calendar event creation '{title}'")
            }
            Self::CalendarCreate { title: None, .. } => "Calendar event creation".to_string(),
            Self::MailDraft { recipients, .. } if !recipients.to.is_empty() => {
                format!("Mail draft creation for '{}'", recipients.to.join(", "))
            }
            Self::MailDraft { .. } => "Mail draft creation".to_string(),
            Self::MailSend { recipients, .. } => {
                format!("Mail send to '{}'", recipients.to.join(", "))
            }
            Self::ConnectedWork {
                connector_name: Some(connector),
                ..
            } => format!("connected work through '{connector}'"),
            Self::ConnectedWork {
                service_binding: Some(binding),
                ..
            } => format!("connected work through {}", binding.label),
            Self::ConnectedWork { .. } => "configured connected work".to_string(),
            Self::ChannelConfigure { platform, .. } => {
                format!("{platform} channel configuration")
            }
        }
    }

    pub(super) fn planner_clause(&self) -> String {
        match self {
            Self::CalendarCreate { instruction, .. }
            | Self::MailDraft { instruction, .. }
            | Self::MailSend { instruction, .. }
            | Self::ConnectedWork { instruction, .. }
            | Self::ChannelConfigure { instruction, .. } => instruction.clone(),
        }
    }
}

pub(super) fn explicit_requirements(objective: &str) -> Vec<CrossSurfaceRequirement> {
    let mut requirements = Vec::new();
    for original_segment in objective_segments(objective) {
        let lowered_segment = original_segment.to_ascii_lowercase();
        let segment = lowered_segment.as_str();
        let recipients = mail_recipients(original_segment);
        let has_recipients =
            !recipients.to.is_empty() || !recipients.cc.is_empty() || !recipients.bcc.is_empty();
        let draft = explicit_mail_draft(segment, has_recipients);
        if draft {
            push_unique(
                &mut requirements,
                CrossSurfaceRequirement::MailDraft {
                    recipients,
                    subject: mail_field(original_segment, "subject"),
                    body: mail_field(original_segment, "body"),
                    instruction: original_segment.trim().to_string(),
                },
            );
        } else if explicit_mail_send(segment, has_recipients) {
            push_unique(
                &mut requirements,
                CrossSurfaceRequirement::MailSend {
                    recipients,
                    subject: mail_field(original_segment, "subject"),
                    body: mail_field(original_segment, "body"),
                    instruction: original_segment.trim().to_string(),
                },
            );
        }

        if calendar_continuation(segment) {
            if !append_calendar_continuation(&mut requirements, original_segment) {
                push_unique(
                    &mut requirements,
                    calendar_requirement(original_segment, true),
                );
            }
        } else if explicit_calendar_create(segment, draft) {
            push_unique(
                &mut requirements,
                calendar_requirement(original_segment, false),
            );
        }
        if explicit_connected_work(segment) {
            push_unique(
                &mut requirements,
                CrossSurfaceRequirement::ConnectedWork {
                    connector_name: connected_field(original_segment, connector_ref_regex()),
                    capability: connected_field(original_segment, capability_regex()),
                    arguments: connected_arguments(original_segment),
                    semantic_tokens: connected_semantic_tokens(original_segment),
                    service_binding: connected_service_binding(segment),
                    instruction: original_segment.trim().to_string(),
                },
            );
        }
        if positive_action_segment(
            segment,
            &[
                "configure",
                "connect",
                "set up",
                "setup",
                "activate",
                "disable",
            ],
        ) {
            for platform in ["telegram", "discord", "slack"] {
                if contains_term(segment, platform) {
                    push_unique(
                        &mut requirements,
                        CrossSurfaceRequirement::ChannelConfigure {
                            platform: platform.to_string(),
                            is_active: !contains_term(segment, "disable"),
                            owner_id: connected_field(original_segment, channel_owner_regex()),
                            instruction: original_segment.trim().to_string(),
                        },
                    );
                }
            }
        }
    }
    requirements
}

pub(super) fn covered(
    requirement: &CrossSurfaceRequirement,
    draft: &GeneratedActionPlanDraft,
    consumed: &mut HashSet<usize>,
) -> bool {
    draft
        .steps
        .iter()
        .enumerate()
        .filter(|(index, _)| !consumed.contains(index))
        .find(|(_, step)| step_covers(requirement, step, draft))
        .map(|(index, _)| consumed.insert(index))
        .unwrap_or(false)
}

pub(super) fn account_binding_deficits(
    objective: &str,
    draft: &GeneratedActionPlanDraft,
    persistence: &crate::db::PersistenceEngine,
    project_id: Option<&str>,
) -> Vec<String> {
    let mut consumed = HashSet::new();
    explicit_requirements(objective)
        .into_iter()
        .filter_map(|requirement| {
            let CrossSurfaceRequirement::ConnectedWork {
                connector_name,
                service_binding,
                ..
            } = &requirement
            else {
                return None;
            };
            if connector_name.is_none() && service_binding.is_none() {
                return None;
            }
            let (index, arguments) = draft
                .steps
                .iter()
                .enumerate()
                .filter(|(index, _)| !consumed.contains(index))
                .find_map(|(index, step)| {
                    let GeneratedToolDraft::RegisteredTaskTool { arguments, .. } = &step.tool
                    else {
                        return None;
                    };
                    step_covers(&requirement, step, draft).then_some((index, arguments))
                })?;
            consumed.insert(index);
            let Some(connector_ref) = arguments.get("connector_ref").and_then(Value::as_str) else {
                return Some(format!(
                    "verified account binding for {}",
                    requirement.label()
                ));
            };
            let Some(capability) = arguments.get("capability").and_then(Value::as_str) else {
                return Some(format!(
                    "verified account binding for {}",
                    requirement.label()
                ));
            };
            persistence
                .validate_planned_connector_authority(
                    connector_ref,
                    service_binding.as_ref().map(|binding| binding.manifest_id),
                    connector_name.as_deref(),
                    project_id,
                    capability,
                )
                .err()
                .map(|_| format!("verified account binding for {}", requirement.label()))
        })
        .collect()
}

fn step_covers(
    requirement: &CrossSurfaceRequirement,
    step: &GeneratedPlanStepDraft,
    draft: &GeneratedActionPlanDraft,
) -> bool {
    let GeneratedToolDraft::RegisteredTaskTool {
        operation,
        arguments,
    } = &step.tool
    else {
        return false;
    };
    let operation = normalized_operation(operation);
    match requirement {
        CrossSurfaceRequirement::CalendarCreate {
            title,
            calendar_name,
            start_minutes_local,
            duration_minutes,
            availability,
            day,
            date,
            location,
            notes,
            authority_conflict,
            ..
        } => {
            !authority_conflict
                && [
                    "create_system_calendar_event",
                    "create_conflict_free_calendar_event",
                    "create_release_recovery_calendar_event",
                    "create_calendar_event",
                ]
                .contains(&operation.as_str())
                && title.as_ref().is_none_or(|expected| {
                    arguments.get("title").and_then(Value::as_str) == Some(expected.as_str())
                })
                && calendar_name.as_ref().is_none_or(|expected| {
                    arguments
                        .get("calendarName")
                        .and_then(Value::as_str)
                        .is_some_and(|actual| actual == expected)
                })
                && start_minutes_local
                    .is_none_or(|expected| calendar_start_matches(arguments, expected))
                && duration_minutes.is_none_or(|expected| {
                    calendar_duration_matches(arguments, draft, &operation, expected)
                })
                && availability.as_ref().is_none_or(|expected| {
                    arguments.get("availability").and_then(Value::as_str) == Some(expected.as_str())
                })
                && day
                    .as_ref()
                    .is_none_or(|expected| calendar_day_matches(arguments, draft, expected))
                && date
                    .as_ref()
                    .is_none_or(|expected| calendar_date_matches(arguments, expected))
                && exact_optional_string(arguments, "location", location)
                && exact_optional_string(arguments, "notes", notes)
        }
        CrossSurfaceRequirement::MailDraft {
            recipients,
            subject,
            body,
            ..
        } => {
            [
                "draft_system_email",
                "draft_decision_pack_email",
                "draft_release_recovery_email",
                "create_mail_draft",
                "draft_email",
            ]
            .contains(&operation.as_str())
                && recipients_match(arguments, recipients)
                && mail_content_matches(arguments, subject, body)
        }
        CrossSurfaceRequirement::MailSend {
            recipients,
            subject,
            body,
            ..
        } => {
            operation == "send_system_email"
                && recipients_match(arguments, recipients)
                && mail_content_matches(arguments, subject, body)
        }
        CrossSurfaceRequirement::ConnectedWork {
            connector_name,
            capability,
            arguments: expected_arguments,
            semantic_tokens,
            service_binding,
            ..
        } => {
            operation == "connected_work"
                && (connector_name.is_none() && service_binding.is_none() || {
                    arguments
                        .get("connector_ref")
                        .and_then(Value::as_str)
                        .is_some_and(|value| !value.trim().is_empty())
                })
                && exact_optional_string(arguments, "capability", capability)
                && expected_arguments
                    .as_ref()
                    .is_none_or(|expected| arguments.get("arguments") == Some(expected))
                && semantic_action_matches(arguments, semantic_tokens)
                && service_binding.as_ref().is_none_or(|binding| {
                    binding.allowed_capabilities.is_empty()
                        || arguments
                            .get("capability")
                            .and_then(Value::as_str)
                            .is_some_and(|capability| {
                                binding.allowed_capabilities.contains(&capability)
                            })
                })
        }
        CrossSurfaceRequirement::ChannelConfigure {
            platform,
            is_active,
            owner_id,
            ..
        } => {
            operation == "configure_channel"
                && arguments
                    .get("platform")
                    .and_then(Value::as_str)
                    .is_some_and(|actual| actual.eq_ignore_ascii_case(platform))
                && arguments.get("is_active").and_then(Value::as_bool) == Some(*is_active)
                && exact_optional_string(arguments, "owner_id", owner_id)
        }
    }
}

fn mail_recipients(segment: &str) -> MailRecipients {
    let mut recipients = MailRecipients::default();
    for found in email_regex().find_iter(segment) {
        let recipient = found.as_str().to_ascii_lowercase();
        match recipient_field(segment, found.start()).as_str() {
            "cc" => recipients.cc.push(recipient),
            "bcc" => recipients.bcc.push(recipient),
            _ => recipients.to.push(recipient),
        }
    }
    normalize_recipient_set(&mut recipients.to);
    normalize_recipient_set(&mut recipients.cc);
    normalize_recipient_set(&mut recipients.bcc);
    recipients
}

fn recipients_match(arguments: &Value, expected: &MailRecipients) -> bool {
    if expected == &MailRecipients::default() {
        return true;
    }
    [
        ("to", &expected.to),
        ("cc", &expected.cc),
        ("bcc", &expected.bcc),
    ]
    .into_iter()
    .all(|(field, expected)| actual_recipient_set(arguments, field) == *expected)
}

fn actual_recipient_set(arguments: &Value, field: &str) -> Vec<String> {
    let mut recipients = arguments
        .get(field)
        .and_then(Value::as_str)
        .into_iter()
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    normalize_recipient_set(&mut recipients);
    recipients
}

fn normalize_recipient_set(recipients: &mut Vec<String>) {
    recipients.sort_unstable();
    recipients.dedup();
}

fn exact_optional_string(arguments: &Value, field: &str, expected: &Option<String>) -> bool {
    expected.as_ref().is_none_or(|expected| {
        arguments.get(field).and_then(Value::as_str) == Some(expected.as_str())
    })
}

fn mail_content_matches(
    arguments: &Value,
    subject: &Option<String>,
    body: &Option<String>,
) -> bool {
    [("subject", subject), ("body", body)]
        .into_iter()
        .all(|(field, expected)| {
            expected.as_ref().is_none_or(|expected| {
                arguments.get(field).and_then(Value::as_str) == Some(expected.as_str())
            })
        })
}

fn calendar_start_matches(arguments: &Value, expected: u16) -> bool {
    arguments
        .get("startDate")
        .and_then(Value::as_str)
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .is_some_and(|value| (value.hour() * 60 + value.minute()) as u16 == expected)
}

fn calendar_duration_matches(
    arguments: &Value,
    draft: &GeneratedActionPlanDraft,
    operation: &str,
    expected: u16,
) -> bool {
    arguments
        .get("durationMinutes")
        .and_then(Value::as_u64)
        .is_some_and(|actual| actual == u64::from(expected))
        || calendar_rfc3339_duration(arguments) == Some(i64::from(expected))
        || (operation == "create_release_recovery_calendar_event"
            && draft.steps.iter().any(|step| {
                matches!(
                    &step.tool,
                    GeneratedToolDraft::RegisteredTaskTool { operation, arguments }
                        if normalized_operation(operation) == "prepare_release_recovery_agenda"
                            && arguments.get("durationMinutes").and_then(Value::as_u64)
                                == Some(u64::from(expected))
                )
            }))
}

fn calendar_rfc3339_duration(arguments: &Value) -> Option<i64> {
    let start = chrono::DateTime::parse_from_rfc3339(arguments.get("startDate")?.as_str()?).ok()?;
    let end = chrono::DateTime::parse_from_rfc3339(arguments.get("endDate")?.as_str()?).ok()?;
    Some((end - start).num_minutes())
}

fn calendar_day_matches(
    arguments: &Value,
    draft: &GeneratedActionPlanDraft,
    expected: &str,
) -> bool {
    if arguments.get("day").and_then(Value::as_str) == Some(expected) {
        return true;
    }
    if draft.steps.iter().any(|step| {
        matches!(
            &step.tool,
            GeneratedToolDraft::RegisteredTaskTool { operation, arguments }
                if normalized_operation(operation) == "prepare_release_recovery_agenda"
                    && arguments.get("day").and_then(Value::as_str) == Some(expected)
        )
    }) {
        return true;
    }
    let Some(date) = calendar_start_date(arguments) else {
        return false;
    };
    match expected {
        "next_weekday" => date == next_weekday(Local::now().date_naive()),
        value if value.starts_with("next_") => weekday_from_name(&value[5..])
            .is_some_and(|weekday| date == next_named_weekday(Local::now().date_naive(), weekday)),
        value => weekday_from_name(value).is_some_and(|weekday| date.weekday() == weekday),
    }
}

fn calendar_date_matches(arguments: &Value, expected: &str) -> bool {
    arguments.get("date").and_then(Value::as_str) == Some(expected)
        || calendar_start_date(arguments)
            .is_some_and(|date| date.format("%Y-%m-%d").to_string() == expected)
}

fn calendar_start_date(arguments: &Value) -> Option<NaiveDate> {
    chrono::DateTime::parse_from_rfc3339(arguments.get("startDate")?.as_str()?)
        .ok()
        .map(|value| value.date_naive())
}

fn next_weekday(mut date: NaiveDate) -> NaiveDate {
    loop {
        date += Duration::days(1);
        if !matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
            return date;
        }
    }
}

fn next_named_weekday(mut date: NaiveDate, weekday: Weekday) -> NaiveDate {
    loop {
        date += Duration::days(1);
        if date.weekday() == weekday {
            return date;
        }
    }
}

fn weekday_from_name(value: &str) -> Option<Weekday> {
    match value {
        "monday" => Some(Weekday::Mon),
        "tuesday" => Some(Weekday::Tue),
        "wednesday" => Some(Weekday::Wed),
        "thursday" => Some(Weekday::Thu),
        "friday" => Some(Weekday::Fri),
        "saturday" => Some(Weekday::Sat),
        "sunday" => Some(Weekday::Sun),
        _ => None,
    }
}

fn recipient_field(segment: &str, address_start: usize) -> String {
    let prefix = &segment[..address_start];
    ["to", "cc", "bcc"]
        .into_iter()
        .filter_map(|field| {
            term_positions(prefix, field)
                .last()
                .map(|position| (position, field))
        })
        .max_by_key(|(position, _)| *position)
        .map(|(_, field)| field.to_string())
        .unwrap_or_else(|| "to".to_string())
}

fn calendar_start_minutes(segment: &str) -> Option<u16> {
    let captures = calendar_start_regex().captures(segment)?;
    let mut hour = captures.get(1)?.as_str().parse::<u16>().ok()?;
    let minute = captures
        .get(2)
        .map_or(Ok(0), |value| value.as_str().parse::<u16>())
        .ok()?;
    let meridiem = captures.get(3)?.as_str();
    if hour == 0 || hour > 12 || minute > 59 {
        return None;
    }
    if hour == 12 {
        hour = 0;
    }
    if meridiem.eq_ignore_ascii_case("pm") {
        hour += 12;
    }
    Some(hour * 60 + minute)
}

fn calendar_start_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?i)\bat\s+(\d{1,2})(?::(\d{2}))?\s*(am|pm)\b")
            .expect("cross-surface Calendar start-time regex")
    })
}

fn calendar_duration_minutes(segment: &str) -> Option<u16> {
    calendar_duration_regex()
        .captures(segment)?
        .get(1)?
        .as_str()
        .parse::<u16>()
        .ok()
        .filter(|value| *value > 0)
}

fn calendar_duration_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?i)\b(\d{1,4})\s*(?:-|\s)\s*minutes?\b")
            .expect("cross-surface Calendar duration regex")
    })
}

fn calendar_availability(segment: &str) -> Option<String> {
    ["tentative", "busy", "free"]
        .into_iter()
        .find(|value| contains_term(segment, value))
        .map(str::to_string)
}

fn calendar_day(segment: &str) -> Option<String> {
    if contains_term(segment, "next weekday") {
        return Some("next_weekday".to_string());
    }
    let captures = calendar_day_regex().captures(segment)?;
    let prefix = captures.get(1).map(|value| value.as_str());
    let weekday = captures.get(2)?.as_str().to_ascii_lowercase();
    Some(if prefix.is_some() {
        format!("next_{weekday}")
    } else {
        weekday
    })
}

fn calendar_day_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?:(next)\s+|on\s+)(monday|tuesday|wednesday|thursday|friday|saturday|sunday)\b",
        )
        .expect("cross-surface Calendar day regex")
    })
}

fn calendar_date(segment: &str) -> Option<String> {
    calendar_date_regex()
        .find(segment)
        .map(|value| value.as_str().to_string())
}

fn calendar_date_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"\b\d{4}-\d{2}-\d{2}\b").expect("cross-surface Calendar date regex")
    })
}

fn calendar_text_field(value: &str, field: &str) -> Option<String> {
    let captures = match field {
        "location" => calendar_location_regex().captures(value)?,
        "notes" => calendar_notes_regex().captures(value)?,
        _ => return None,
    };
    (1..captures.len())
        .find_map(|index| captures.get(index))
        .map(|found| found.as_str().trim().to_string())
        .filter(|value| !value.is_empty())
}

fn calendar_location_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r#"(?i)\blocation\s*:?\s*(?:`([^`\r\n]+)`|\"([^\"\r\n]+)\"|([^,.;\r\n]+?)(?:\s+with\s+notes?\b|$))"#,
        )
        .expect("cross-surface Calendar location regex")
    })
}

fn calendar_notes_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r#"(?i)\bnotes?\s*:?\s*(?:`([^`\r\n]+)`|\"([^\"\r\n]+)\"|([^,.;\r\n]+?)(?:\s+and\s+mark\b|$))"#,
        )
        .expect("cross-surface Calendar notes regex")
    })
}

fn explicit_calendar_create(segment: &str, mail_draft: bool) -> bool {
    positive_action_segment(segment, &["create", "add", "schedule", "book", "propose"])
        && (contains_term(segment, "event")
            || contains_term(segment, "appointment")
            || (!mail_draft && contains_term(segment, "meeting")))
}

fn calendar_continuation(segment: &str) -> bool {
    contains_any(
        segment,
        &[
            "schedule it",
            "schedule the event",
            "reschedule it",
            "reschedule the event",
            "that event",
            "the same event",
        ],
    )
}

fn calendar_requirement(
    original_segment: &str,
    authority_conflict: bool,
) -> CrossSurfaceRequirement {
    let segment = original_segment.to_ascii_lowercase();
    CrossSurfaceRequirement::CalendarCreate {
        title: event_title(original_segment),
        calendar_name: calendar_name(original_segment),
        start_minutes_local: calendar_start_minutes(&segment),
        duration_minutes: calendar_duration_minutes(&segment),
        availability: calendar_availability(&segment),
        day: calendar_day(&segment),
        date: calendar_date(&segment),
        location: calendar_text_field(original_segment, "location"),
        notes: calendar_text_field(original_segment, "notes"),
        authority_conflict,
        instruction: original_segment.trim().to_string(),
    }
}

fn append_calendar_continuation(
    requirements: &mut [CrossSurfaceRequirement],
    original_segment: &str,
) -> bool {
    let incoming = calendar_requirement(original_segment, false);
    let CrossSurfaceRequirement::CalendarCreate {
        title: incoming_title,
        calendar_name: incoming_calendar_name,
        start_minutes_local: incoming_start,
        duration_minutes: incoming_duration,
        availability: incoming_availability,
        day: incoming_day,
        date: incoming_date,
        location: incoming_location,
        notes: incoming_notes,
        ..
    } = incoming
    else {
        unreachable!("calendar_requirement returns CalendarCreate")
    };
    let Some(CrossSurfaceRequirement::CalendarCreate {
        title,
        calendar_name,
        start_minutes_local,
        duration_minutes,
        availability,
        day,
        date,
        location,
        notes,
        authority_conflict,
        instruction,
    }) = requirements
        .iter_mut()
        .rev()
        .find(|requirement| matches!(requirement, CrossSurfaceRequirement::CalendarCreate { .. }))
    else {
        return false;
    };
    merge_authority(title, incoming_title, authority_conflict);
    merge_authority(calendar_name, incoming_calendar_name, authority_conflict);
    merge_authority(start_minutes_local, incoming_start, authority_conflict);
    merge_authority(duration_minutes, incoming_duration, authority_conflict);
    merge_authority(availability, incoming_availability, authority_conflict);
    merge_authority(day, incoming_day, authority_conflict);
    merge_authority(date, incoming_date, authority_conflict);
    merge_authority(location, incoming_location, authority_conflict);
    merge_authority(notes, incoming_notes, authority_conflict);
    if !instruction.ends_with(['.', '!', '?']) {
        instruction.push('.');
    }
    instruction.push(' ');
    instruction.push_str(original_segment.trim());
    true
}

fn merge_authority<T: Eq>(target: &mut Option<T>, incoming: Option<T>, conflict: &mut bool) {
    let Some(incoming) = incoming else {
        return;
    };
    match target {
        Some(current) if current != &incoming => *conflict = true,
        Some(_) => {}
        None => *target = Some(incoming),
    }
}

fn explicit_mail_draft(segment: &str, has_recipient: bool) -> bool {
    let mail_context =
        contains_term(segment, "mail") || contains_term(segment, "email") || has_recipient;
    let mail_position = ["mail", "email"]
        .iter()
        .filter_map(|term| term_positions(segment, term).next())
        .min();
    let draft_verb = mail_position.is_some_and(|mail| {
        term_positions(segment, "draft")
            .any(|draft| draft < mail && !action_is_negated(segment, draft))
    });
    mail_context
        && (contains_term(segment, "draft") || contains_term(segment, "unsent"))
        && (positive_action_segment(segment, &["create", "compose", "prepare", "write"])
            || draft_verb)
}

fn explicit_mail_send(segment: &str, has_recipient: bool) -> bool {
    if !has_recipient
        || contains_term(segment, "draft")
        || contains_term(segment, "unsent")
        || segment.contains("do not send")
        || segment.contains("don't send")
        || segment.contains("never send")
    {
        return false;
    }
    positive_action_segment(segment, &["send", "email", "mail"])
}

fn explicit_connected_work(segment: &str) -> bool {
    let has_recipient = email_regex().is_match(segment);
    let mail_draft = explicit_mail_draft(segment, has_recipient);
    if (contains_term(segment, "calendar") || segment.contains("my calendars"))
        && (segment.contains("conflict-free") || explicit_calendar_create(segment, mail_draft))
    {
        return false;
    }
    if contains_any(
        segment,
        &[
            "do not use",
            "don't use",
            "never use",
            "without using",
            "do not access",
            "don't access",
            "never access",
            "without accessing",
            "do not read",
            "don't read",
            "never read",
            "without reading",
            "do not check",
            "don't check",
            "never check",
            "without checking",
        ],
    ) {
        return false;
    }
    let without_email_addresses = email_regex().replace_all(segment, " ");
    let service = contains_any(
        &without_email_addresses,
        &[
            "mcp server",
            "configured mcp",
            "connected account",
            "connected service",
            "apple apps",
            "apple app connector",
            "connected connector",
            "configured connector",
            "apple mail",
            "mail app",
            "my mail",
            "my email",
            "my emails",
            "my inbox",
            "mail inbox",
            "email inbox",
            "unread email",
            "unread mail",
            "apple calendar",
            "calendar app",
            "my calendar",
            "google drive",
            "google calendar",
            "gmail",
            "microsoft 365",
            "outlook",
            "notion",
            "slack",
            "teams",
        ],
    );
    let action = contains_any(
        segment,
        &[
            "use ",
            "using ",
            "access ",
            "read ",
            "check ",
            "find ",
            "search ",
            "list ",
            "show ",
            "retrieve ",
            "review ",
            "summarize ",
            "summarise ",
            "open ",
            "look up ",
            "create ",
            "update ",
            "add ",
            "schedule ",
            "send ",
            "draft ",
            "do i have ",
            "do we have ",
            "are there any ",
            "what is on my ",
            "what's on my ",
        ],
    );
    service && action
}

fn connected_service_binding(segment: &str) -> Option<ConnectedServiceBinding> {
    const MAIL_READ: &[&str] = &["find_email", "read_email"];
    const MAIL_DRAFT: &[&str] = &["draft_email"];
    const CALENDAR_READ: &[&str] = &["read_calendar"];
    const CALENDAR_DRAFT: &[&str] = &["draft_calendar_event"];
    const PERSONAL_FILES_READ: &[&str] = &["find_personal_files", "read_personal_file"];
    const PERSONAL_FILES_WRITE: &[&str] = &["save_personal_file"];
    const CHATS_READ: &[&str] = &["list_chats", "find_chat_messages"];
    const CHATS_DRAFT: &[&str] = &["draft_chat_message"];
    const ALL_READ: &[&str] = &[
        "find_email",
        "read_email",
        "read_calendar",
        "find_personal_files",
        "read_personal_file",
        "find_team_files",
        "read_team_file",
        "find_team_site",
        "list_chats",
        "find_chat_messages",
    ];
    let binding = |manifest_id, allowed_capabilities, label| ConnectedServiceBinding {
        manifest_id,
        allowed_capabilities,
        label,
    };
    let draft = positive_action_segment(segment, &["draft", "compose"])
        || (positive_action_segment(segment, &["prepare"])
            && contains_any(
                segment,
                &["email", "mail", "message", "reply", "event", "appointment"],
            ));
    let save = positive_action_segment(segment, &["save", "write", "upload"])
        || (positive_action_segment(segment, &["create"])
            && contains_any(segment, &["file", "document"]));
    if contains_term(segment, "apple mail") {
        return Some(binding(
            "apple_apps",
            if draft { MAIL_DRAFT } else { MAIL_READ },
            "Apple Mail",
        ));
    }
    if contains_term(segment, "apple calendar") {
        return Some(binding(
            "apple_apps",
            if draft { CALENDAR_DRAFT } else { CALENDAR_READ },
            "Apple Calendar",
        ));
    }
    if contains_term(segment, "gmail") {
        return Some(binding(
            "google_workspace",
            if draft { MAIL_DRAFT } else { MAIL_READ },
            "Gmail",
        ));
    }
    if contains_any(
        segment,
        &[
            "mail app",
            "my mail",
            "my email",
            "my emails",
            "my inbox",
            "mail inbox",
            "email inbox",
            "unread email",
            "unread mail",
        ],
    ) {
        return Some(binding(
            "apple_apps",
            if draft { MAIL_DRAFT } else { MAIL_READ },
            "Apple Mail",
        ));
    }
    if contains_term(segment, "google calendar") {
        return Some(binding(
            "google_workspace",
            if draft { CALENDAR_DRAFT } else { CALENDAR_READ },
            "Google Calendar",
        ));
    }
    if contains_term(segment, "calendar app") || contains_term(segment, "my calendar") {
        return Some(binding(
            "apple_apps",
            if draft { CALENDAR_DRAFT } else { CALENDAR_READ },
            "Apple Calendar",
        ));
    }
    if contains_term(segment, "google drive") {
        return Some(binding(
            "google_workspace",
            if save {
                PERSONAL_FILES_WRITE
            } else {
                PERSONAL_FILES_READ
            },
            "Google Drive",
        ));
    }
    if contains_term(segment, "teams") {
        return Some(binding(
            "microsoft_365",
            if draft { CHATS_DRAFT } else { CHATS_READ },
            "Microsoft Teams",
        ));
    }
    if contains_term(segment, "outlook") {
        return Some(if contains_term(segment, "calendar") {
            binding(
                "microsoft_365",
                if draft { CALENDAR_DRAFT } else { CALENDAR_READ },
                "Outlook Calendar",
            )
        } else {
            binding(
                "microsoft_365",
                if draft { MAIL_DRAFT } else { MAIL_READ },
                "Outlook Mail",
            )
        });
    }
    if contains_term(segment, "microsoft 365") {
        return Some(binding("microsoft_365", ALL_READ, "Microsoft 365"));
    }
    if contains_term(segment, "slack") {
        return Some(binding(
            "slack",
            if draft { CHATS_DRAFT } else { CHATS_READ },
            "Slack",
        ));
    }
    if contains_term(segment, "notion") {
        return Some(binding("notion", ALL_READ, "Notion"));
    }
    if contains_term(segment, "mcp server") || contains_term(segment, "configured mcp") {
        return Some(binding("mcp_runtime", &[], "the requested MCP server"));
    }
    if contains_term(segment, "apple app connector") || contains_term(segment, "apple apps") {
        return Some(binding("apple_apps", ALL_READ, "Apple Apps"));
    }
    None
}

fn event_title(segment: &str) -> Option<String> {
    let captures = event_title_regex().captures(segment)?;
    captures
        .get(1)
        .or_else(|| captures.get(2))
        .map(|found| {
            let title = found.as_str().trim();
            title
                .to_ascii_lowercase()
                .find(" in my ")
                .map(|boundary| &title[..boundary])
                .unwrap_or(title)
                .trim()
                .to_string()
        })
        .filter(|value| !value.is_empty())
}

fn event_title_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r#"(?i)\b(?:event|meeting|appointment)\s+titled\s+(?:`([^`\r\n]{1,160})`|([^,.\r\n]{1,160}))"#,
        )
        .expect("cross-surface event title regex")
    })
}

fn calendar_name(value: &str) -> Option<String> {
    let captures = calendar_name_regex().captures(value)?;
    captures
        .get(1)
        .or_else(|| captures.get(2))
        .map(|found| found.as_str().trim().to_string())
        .filter(|value| !value.is_empty())
}

fn calendar_name_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?i)\bin my\s+(?:`([^`\r\n]+)`|([a-z0-9][a-z0-9 _-]{0,120}?))\s+calendar\b")
            .expect("cross-surface Calendar name regex")
    })
}

fn connected_field(value: &str, regex: &Regex) -> Option<String> {
    let captures = regex.captures(value)?;
    (1..captures.len())
        .find_map(|index| captures.get(index))
        .map(|found| found.as_str().trim().to_string())
        .filter(|value| !value.is_empty())
}

fn connector_ref_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r#"(?i)\b(?:mcp server|connector)\s+(?:named|called)\s+(?:`([^`\r\n]+)`|\"([^\"\r\n]+)\"|([a-z0-9_.:-]+))"#,
        )
        .expect("cross-surface connector reference regex")
    })
}

fn capability_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r#"(?i)\bcapability\s+(?:named\s+)?(?:`([^`\r\n]+)`|\"([^\"\r\n]+)\"|([a-z0-9_.:-]+))"#,
        )
        .expect("cross-surface connector capability regex")
    })
}

fn channel_owner_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r#"(?i)\b(?:for\s+)?owner(?:_id)?\s+(?:is\s+)?(?:`([^`\r\n]*)`|\"([^\"\r\n]*)\"|([a-z0-9_.:@+-]+))"#,
        )
        .expect("cross-surface channel owner regex")
    })
}

fn connected_arguments(value: &str) -> Option<Value> {
    let raw = connected_arguments_regex()
        .captures(value)?
        .get(1)?
        .as_str()
        .trim_matches('`');
    serde_json::from_str(raw).ok()
}

fn connected_arguments_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r#"(?i)\barguments?\s+(`?\{[^}\r\n]{0,2000}\}`?)"#)
            .expect("cross-surface connector arguments regex")
    })
}

fn connected_semantic_tokens(value: &str) -> Vec<String> {
    if connected_field(value, capability_regex()).is_some() || connected_arguments(value).is_some()
    {
        return Vec::new();
    }
    let Some(connector_match) = connector_ref_regex().find(value) else {
        return Vec::new();
    };
    let suffix = &value[connector_match.end()..];
    let action = semantic_action_prefix_regex()
        .captures(suffix)
        .and_then(|captures| captures.get(1))
        .map(|found| found.as_str())
        .unwrap_or("");
    let tokens = normalized_semantic_tokens(action);
    if tokens.is_empty() {
        vec!["__unresolved_connector_action__".to_string()]
    } else {
        tokens
    }
}

fn semantic_action_prefix_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?is)^\s*(?:to|for)\s+(.+)$").expect("cross-surface connector action regex")
    })
}

fn semantic_action_matches(arguments: &Value, expected: &[String]) -> bool {
    if expected.is_empty() {
        return true;
    }
    let mut source = arguments
        .get("capability")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if let Some(payload) = arguments.get("arguments") {
        source.push(' ');
        source.push_str(&payload.to_string());
    }
    let actual = normalized_semantic_tokens(&source)
        .into_iter()
        .collect::<HashSet<_>>();
    expected.iter().all(|token| actual.contains(token))
}

fn normalized_semantic_tokens(value: &str) -> Vec<String> {
    semantic_word_regex()
        .find_iter(value)
        .map(|found| normalize_semantic_token(found.as_str()))
        .filter(|token| !semantic_stopword(token))
        .take(16)
        .collect()
}

fn semantic_word_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?i)[a-z0-9]+").expect("cross-surface connector semantic-token regex")
    })
}

fn normalize_semantic_token(value: &str) -> String {
    match value.to_ascii_lowercase().as_str() {
        "fetch" | "find" | "get" | "lookup" | "read" | "search" => "retrieve".to_string(),
        value => value.to_string(),
    }
}

fn semantic_stopword(value: &str) -> bool {
    matches!(
        value,
        "a" | "an" | "and" | "for" | "from" | "in" | "my" | "of" | "on" | "the" | "to" | "with"
    )
}

fn mail_field(value: &str, field: &str) -> Option<String> {
    let max = match field {
        "subject" => 998,
        "body" => 12_000,
        _ => return None,
    };
    let lowered = value.to_ascii_lowercase();
    let start = term_positions(&lowered, field).next()? + field.len();
    let remainder = value.get(start..)?.trim_start();
    if let Some(quoted) = quoted_prefix(remainder) {
        return (quoted.chars().count() <= max).then_some(quoted);
    }
    let lowered_remainder = remainder.to_ascii_lowercase();
    let boundaries: &[&str] = if field == "subject" {
        &[
            " and the report",
            " and report",
            " and attach",
            " and include",
            " and link",
            " and body",
            " with body",
            " with attachment",
            ".",
            ";",
            "\r",
            "\n",
        ]
    } else {
        &[
            " and attach",
            " and include",
            " and link",
            " and send",
            " and create",
            " and schedule",
            " and verify",
            ".",
            ";",
            "\r",
            "\n",
        ]
    };
    let end = boundaries
        .iter()
        .filter_map(|boundary| lowered_remainder.find(boundary))
        .min()
        .unwrap_or(remainder.len());
    let extracted = remainder[..end].trim();
    (!extracted.is_empty() && extracted.chars().count() <= max).then(|| extracted.to_string())
}

fn quoted_prefix(value: &str) -> Option<String> {
    let quote = value.chars().next()?;
    if !matches!(quote, '`' | '"') {
        return None;
    }
    let content = &value[quote.len_utf8()..];
    let end = content.find(quote)?;
    let content = content[..end].trim();
    (!content.is_empty()).then(|| content.to_string())
}

fn email_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?i)\b[a-z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-z0-9.-]+\.[a-z]{2,}\b")
            .expect("cross-surface email regex")
    })
}

fn objective_segments(value: &str) -> impl Iterator<Item = &str> {
    value
        .split(['\n', ';'])
        .flat_map(|segment| segment.split(". "))
        .flat_map(|segment| segment.split("? "))
        .flat_map(|segment| segment.split("! "))
}

fn contains_any(value: &str, terms: &[&str]) -> bool {
    terms.iter().any(|term| value.contains(term))
}

fn push_unique(
    requirements: &mut Vec<CrossSurfaceRequirement>,
    requirement: CrossSurfaceRequirement,
) {
    if !requirements.contains(&requirement) {
        requirements.push(requirement);
    }
}

#[cfg(test)]
#[path = "compound_requirements_tests.rs"]
mod tests;
