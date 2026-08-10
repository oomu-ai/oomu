use regex::Regex;
use std::sync::OnceLock;

use super::{ChatIntentRoute, ChatIntentRouteDecision};
#[path = "future_schedule_cadence.rs"]
mod cadence;
#[path = "future_schedule_family.rs"]
mod family;
use cadence::{
    named_cadence_looks_like_noun, recurring_cadence, unsupported_recurring_cadence, CadenceUnit,
    RecurringCadence, UnsupportedCadence,
};
use family::{recurring_schedule_family, RecurringScheduleFamily};

pub(super) fn future_schedule_decision(prompt: &str) -> Option<ChatIntentRouteDecision> {
    let schedule = scheduled_action_request(prompt)?;
    let mut matched_signals = schedule.signals();
    if let Some(app_kind) = crate::local_app_intent::private_app_data_kind(prompt) {
        matched_signals.push(format!("routine target private app:v1:{app_kind}"));
    }
    if schedule.run_once_requested {
        matched_signals.push("explicit run once requested".to_string());
    }
    if schedule.ends_at_midnight {
        matched_signals.push("end at midnight requested".to_string());
    }
    Some(ChatIntentRouteDecision {
        route: ChatIntentRoute::AgenticPlanner,
        requires_local_access: true,
        decision_source: "routine_scheduler_filter".to_string(),
        reason: schedule.reason(),
        matched_signals,
        status_label: "OOMU is preparing the schedule...".to_string(),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScheduleKind {
    OneTime,
    Recurring(RecurringCadence),
    RecurringFamily(RecurringScheduleFamily),
    UnsupportedRecurring(UnsupportedCadence),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ScheduledActionRequest {
    kind: ScheduleKind,
    run_once_requested: bool,
    ends_at_midnight: bool,
    explicit_daily_time: Option<(u8, u8)>,
}

impl ScheduledActionRequest {
    fn signals(self) -> Vec<String> {
        match self.kind {
            ScheduleKind::OneTime => vec!["future one-time routine".to_string()],
            ScheduleKind::Recurring(cadence) => {
                let mut signals = vec![
                    "recurring routine".to_string(),
                    cadence.signal(),
                    format!(
                        "routine schedule seed: {}",
                        self.explicit_daily_time
                            .map(daily_time_seed)
                            .unwrap_or_else(|| cadence.schedule_seed())
                    ),
                ];
                if cadence.unit.needs_calendar_default() && self.explicit_daily_time.is_none() {
                    signals.push("routine timing defaulted".to_string());
                }
                signals
            }
            ScheduleKind::RecurringFamily(family) => {
                let mut signals = vec![
                    "recurring routine".to_string(),
                    family.cadence.signal(),
                    format!(
                        "routine schedule seed: {}",
                        self.explicit_daily_time
                            .map(daily_time_seed)
                            .unwrap_or_else(|| family.schedule_seed.to_string())
                    ),
                ];
                if self.explicit_daily_time.is_none() {
                    signals.push("routine timing defaulted".to_string());
                }
                signals
            }
            ScheduleKind::UnsupportedRecurring(cadence) => vec![
                "recurring routine".to_string(),
                format!("routine schedule seed: {}", cadence.schedule_seed()),
                "routine schedule unsupported".to_string(),
                "routine schedule clarification required".to_string(),
            ],
        }
    }

    fn reason(self) -> String {
        match self.kind {
            ScheduleKind::OneTime => "The prompt assigns work to a future time, so OOMU must prepare a one-time Routine instead of running the action now."
                .to_string(),
            ScheduleKind::Recurring(cadence)
            | ScheduleKind::RecurringFamily(RecurringScheduleFamily { cadence, .. })
                if self.ends_at_midnight
                    && cadence.unit.conflicts_with_same_day_midnight() =>
            {
                format!(
                    "The prompt requests work every {} {} only until midnight today. Those limits leave no future recurrence at that cadence; OOMU must preserve the midnight stop in review and cannot claim a recurring run will occur. Any explicit test run remains unexecuted until confirmation.",
                    cadence.interval,
                    cadence.unit.as_str(),
                )
            }
            ScheduleKind::Recurring(cadence)
            | ScheduleKind::RecurringFamily(RecurringScheduleFamily { cadence, .. })
                if self.ends_at_midnight =>
            {
                let run_once = if self.run_once_requested {
                    " The requested immediate test run remains unexecuted until confirmation."
                } else {
                    ""
                };
                format!(
                    "The prompt requests work every {} {} with an enforced midnight stop. OOMU must open Routine review before activation.{run_once}",
                    cadence.interval,
                    cadence.unit.as_str(),
                )
            }
            ScheduleKind::Recurring(cadence)
            | ScheduleKind::RecurringFamily(RecurringScheduleFamily { cadence, .. })
                if cadence.unit.needs_calendar_default()
                    && self.explicit_daily_time.is_none() =>
            {
                let run_once = if self.run_once_requested {
                    " The requested immediate test run remains unexecuted until confirmation."
                } else {
                    ""
                };
                format!(
                    "The prompt requests work every {} {} without complete calendar timing. OOMU must open Routine review with an explicit editable default instead of inventing a completed schedule.{run_once}",
                    cadence.interval,
                    cadence.unit.as_str(),
                )
            }
            ScheduleKind::Recurring(cadence)
            | ScheduleKind::RecurringFamily(RecurringScheduleFamily { cadence, .. }) => {
                let run_once = if self.run_once_requested {
                    " The requested immediate test run remains unexecuted until confirmation."
                } else {
                    ""
                };
                format!(
                    "The prompt requests work every {} {}. OOMU must open Routine review instead of scheduling it without confirmation.{run_once}",
                    cadence.interval,
                    cadence.unit.as_str(),
                )
            }
            ScheduleKind::UnsupportedRecurring(UnsupportedCadence::SubMinute(_)) =>
                "The prompt requests a sub-minute cadence. OOMU must open Routine review and ask for a supported interval of at least one minute instead of silently ignoring the schedule or claiming it can run."
                    .to_string(),
            ScheduleKind::UnsupportedRecurring(UnsupportedCadence::Vague) =>
                "The prompt requests recurring work without a concrete interval. OOMU must open Routine review and ask for an explicit count and time unit instead of inventing a schedule."
                    .to_string(),
            ScheduleKind::UnsupportedRecurring(
                UnsupportedCadence::Biweekly | UnsupportedCadence::Bimonthly,
            ) => "The prompt uses a cadence label with more than one common meaning. OOMU must open Routine review and ask for an explicit count and time unit instead of choosing an interpretation."
                .to_string(),
        }
    }
}

fn scheduled_action_request(prompt: &str) -> Option<ScheduledActionRequest> {
    let normalized = prompt
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    if normalized.is_empty()
        || normalized.contains("what is on my schedule")
        || normalized.contains("what's on my schedule")
        || normalized.contains("show my schedule")
    {
        return None;
    }
    let padded = format!(" {normalized} ");
    let has_action = [
        " check ",
        " read ",
        " review ",
        " run ",
        " create ",
        " write ",
        " tell ",
        " verify ",
        " look ",
        " inspect ",
        " summarize ",
        " summarise ",
        " schedule ",
    ]
    .iter()
    .any(|term| padded.contains(term));

    if !has_action {
        return None;
    }

    let explicit_schedule = padded.contains(" schedule ")
        || normalized.contains("set up")
        || normalized.contains("every ")
        || normalized.contains("once per ");
    if named_cadence_looks_like_noun(&normalized) && !explicit_schedule {
        return None;
    }

    if let Some(family) = recurring_schedule_family(&normalized) {
        return Some(ScheduledActionRequest {
            kind: ScheduleKind::RecurringFamily(family),
            run_once_requested: run_once_request_regex()
                .is_some_and(|regex| regex.is_match(&normalized)),
            ends_at_midnight: end_at_midnight_regex()
                .is_some_and(|regex| regex.is_match(&normalized)),
            explicit_daily_time: (family.cadence.unit == CadenceUnit::Day)
                .then(|| explicit_time_of_day(&normalized))
                .flatten(),
        });
    }

    if let Some(cadence) = recurring_cadence(&normalized) {
        return Some(ScheduledActionRequest {
            kind: ScheduleKind::Recurring(cadence),
            run_once_requested: run_once_request_regex()
                .is_some_and(|regex| regex.is_match(&normalized)),
            ends_at_midnight: end_at_midnight_regex()
                .is_some_and(|regex| regex.is_match(&normalized)),
            explicit_daily_time: (cadence.unit == CadenceUnit::Day)
                .then(|| explicit_time_of_day(&normalized))
                .flatten(),
        });
    }

    if let Some(cadence) = unsupported_recurring_cadence(&normalized) {
        return Some(ScheduledActionRequest {
            kind: ScheduleKind::UnsupportedRecurring(cadence),
            run_once_requested: run_once_request_regex()
                .is_some_and(|regex| regex.is_match(&normalized)),
            ends_at_midnight: end_at_midnight_regex()
                .is_some_and(|regex| regex.is_match(&normalized)),
            explicit_daily_time: None,
        });
    }

    future_schedule_regex()
        .is_some_and(|regex| regex.is_match(&normalized))
        .then_some(ScheduledActionRequest {
            kind: ScheduleKind::OneTime,
            run_once_requested: false,
            ends_at_midnight: false,
            explicit_daily_time: None,
        })
}

fn daily_time_seed((hour, minute): (u8, u8)) -> String {
    format!("daily at {hour:02}:{minute:02}")
}

fn explicit_time_of_day(prompt: &str) -> Option<(u8, u8)> {
    let captures = explicit_time_of_day_regex()?.captures(prompt)?;
    let mut hour = captures.name("hour")?.as_str().parse::<u8>().ok()?;
    let minute = captures
        .name("minute")
        .map(|value| value.as_str().parse::<u8>().ok())
        .unwrap_or(Some(0))?;
    let meridiem = captures.name("meridiem")?.as_str();
    if !(1..=12).contains(&hour) || minute > 59 {
        return None;
    }
    if meridiem.starts_with('p') && hour != 12 {
        hour += 12;
    } else if meridiem.starts_with('a') && hour == 12 {
        hour = 0;
    }
    Some((hour, minute))
}

fn explicit_time_of_day_regex() -> Option<&'static Regex> {
    static REGEX: OnceLock<Result<Regex, regex::Error>> = OnceLock::new();
    REGEX
        .get_or_init(|| {
            Regex::new(
                r"(?ix)\bat\s+(?P<hour>1[0-2]|0?[1-9])(?::(?P<minute>[0-5]\d))?\s*(?P<meridiem>a\.?m\.?|p\.?m\.?)\b",
            )
        })
        .as_ref()
        .ok()
}

fn run_once_request_regex() -> Option<&'static Regex> {
    static REGEX: OnceLock<Result<Regex, regex::Error>> = OnceLock::new();
    REGEX
        .get_or_init(|| {
            Regex::new(
                r"(?ix)\b(?:
                    (?:run|do|check|read)\s+(?:it\s+)?(?:once\s+)?(?:right\s+)?now
                    |(?:test[\s-]*run|run|execute|try|check|read)\s+(?:it|the\s+(?:task|schedule|workflow))\s+(?:just\s+)?once
                    |(?:run|execute)\s+once\s+(?:now|right\s+now|to\s+(?:test|check|verify))
                )\b",
            )
        })
        .as_ref()
        .ok()
}

fn end_at_midnight_regex() -> Option<&'static Regex> {
    static REGEX: OnceLock<Result<Regex, regex::Error>> = OnceLock::new();
    REGEX
        .get_or_init(|| {
            Regex::new(
                r"(?ix)\b(?:until|through|ending\s+at|ends?\s+at|stopping\s+at|stops?\s+at)\s+midnight\b",
            )
        })
        .as_ref()
        .ok()
}

fn future_schedule_regex() -> Option<&'static Regex> {
    static REGEX: OnceLock<Result<Regex, regex::Error>> = OnceLock::new();
    REGEX
        .get_or_init(|| {
            Regex::new(
                r"(?ix)
                (?:^|\b)(?:
                    at\s+(?:[01]?\d|2[0-3])(?::[0-5]\d)?\s*(?:am|pm)?\s+(?:today|tomorrow)\s*[,;:-]
                    |tomorrow\s+at\s+(?:[01]?\d|2[0-3])(?::[0-5]\d)?\s*(?:am|pm)?
                    |on\s+20\d{2}-\d{2}-\d{2}\s+at\s+(?:[01]?\d|2[0-3])(?::[0-5]\d)?\s*(?:am|pm)?
                    |in\s+(?:\d+|one|two|three|four|five|six|seven|eight|nine|ten)\s+(?:minutes?|hours?)\b
                    |(?:schedule|run)\b.{0,80}\b(?:at|tomorrow|today|on\s+20\d{2}-\d{2}-\d{2})\b
                )",
            )
        })
        .as_ref()
        .ok()
}
