use regex::Regex;
use std::sync::OnceLock;

use super::cadence::{CadenceUnit, RecurringCadence};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct RecurringScheduleFamily {
    pub(super) cadence: RecurringCadence,
    pub(super) schedule_seed: &'static str,
}

pub(super) fn recurring_schedule_family(prompt: &str) -> Option<RecurringScheduleFamily> {
    let captures = recurring_family_regex()?.captures(prompt)?;
    let raw = captures
        .name("explicit")
        .or_else(|| captures.name("plural"))
        .or_else(|| captures.name("daypart"))?
        .as_str();
    let (unit, schedule_seed) = match raw {
        "weekday" | "weekdays" => (CadenceUnit::Week, "every weekday"),
        "weekend" | "weekends" => (CadenceUnit::Week, "every weekend"),
        "sunday" | "sundays" => (CadenceUnit::Week, "every sunday"),
        "monday" | "mondays" => (CadenceUnit::Week, "every monday"),
        "tuesday" | "tuesdays" => (CadenceUnit::Week, "every tuesday"),
        "wednesday" | "wednesdays" => (CadenceUnit::Week, "every wednesday"),
        "thursday" | "thursdays" => (CadenceUnit::Week, "every thursday"),
        "friday" | "fridays" => (CadenceUnit::Week, "every friday"),
        "saturday" | "saturdays" => (CadenceUnit::Week, "every saturday"),
        "morning" => (CadenceUnit::Day, "every morning"),
        "afternoon" => (CadenceUnit::Day, "every afternoon"),
        "evening" => (CadenceUnit::Day, "every evening"),
        "night" | "nightly" => (CadenceUnit::Day, "every night"),
        _ => return None,
    };
    Some(RecurringScheduleFamily {
        cadence: RecurringCadence { interval: 1, unit },
        schedule_seed,
    })
}

fn recurring_family_regex() -> Option<&'static Regex> {
    static REGEX: OnceLock<Result<Regex, regex::Error>> = OnceLock::new();
    REGEX
        .get_or_init(|| {
            Regex::new(
                r"(?ix)\b(?:
                    (?:every|each)\s+(?P<explicit>weekday|weekend|sunday|monday|tuesday|wednesday|thursday|friday|saturday|morning|afternoon|evening|night)
                    |(?P<plural>weekdays|weekends|sundays|mondays|tuesdays|wednesdays|thursdays|fridays|saturdays|nightly)
                    |every\s+day\s+in\s+the\s+(?P<daypart>morning|afternoon|evening)
                )\b",
            )
        })
        .as_ref()
        .ok()
}
