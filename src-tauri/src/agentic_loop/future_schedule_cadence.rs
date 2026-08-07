use regex::Regex;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CadenceUnit {
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Quarter,
    Year,
}

impl CadenceUnit {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Minute => "minute",
            Self::Hour => "hour",
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
            Self::Quarter => "quarter",
            Self::Year => "year",
        }
    }

    pub(super) fn needs_calendar_default(self) -> bool {
        matches!(
            self,
            Self::Day | Self::Week | Self::Month | Self::Quarter | Self::Year
        )
    }

    pub(super) fn conflicts_with_same_day_midnight(self) -> bool {
        self.needs_calendar_default()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct RecurringCadence {
    pub(super) interval: u32,
    pub(super) unit: CadenceUnit,
}

impl RecurringCadence {
    pub(super) fn signal(self) -> String {
        format!(
            "routine cadence:v1:{}:{}",
            self.interval,
            self.unit.as_str()
        )
    }

    pub(super) fn schedule_seed(self) -> String {
        let unit = self.unit.as_str();
        match (self.interval, self.unit) {
            (1, CadenceUnit::Day) => "every day".to_string(),
            (1, CadenceUnit::Week) => "every week".to_string(),
            (1, CadenceUnit::Month) => "every month".to_string(),
            (1, CadenceUnit::Quarter) => "every quarter".to_string(),
            (1, CadenceUnit::Year) => "every year".to_string(),
            (1, _) => format!("every 1 {unit}"),
            (interval, _) => format!("every {interval} {unit}s"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum UnsupportedCadence {
    SubMinute(u32),
    Biweekly,
    Bimonthly,
    Vague,
}

impl UnsupportedCadence {
    pub(super) fn schedule_seed(self) -> String {
        match self {
            Self::SubMinute(1) => "every 1 second".to_string(),
            Self::SubMinute(interval) => format!("every {interval} seconds"),
            Self::Biweekly => "biweekly".to_string(),
            Self::Bimonthly => "bimonthly".to_string(),
            Self::Vague => "periodically".to_string(),
        }
    }
}

pub(super) fn recurring_cadence(prompt: &str) -> Option<RecurringCadence> {
    let captures = recurring_cadence_regex()?.captures(prompt)?;
    let (count, raw_unit) = if let Some(unit) = captures.name("every_unit") {
        (captured_count(&captures), unit.as_str())
    } else if let Some(unit) = captures.name("once_unit") {
        (1, unit.as_str())
    } else {
        (1, captures.name("named")?.as_str())
    };
    let unit = match raw_unit.trim_end_matches('s') {
        "minute" | "min" | "minutely" => CadenceUnit::Minute,
        "hour" | "hr" | "hourly" => CadenceUnit::Hour,
        "day" | "daily" => CadenceUnit::Day,
        "week" | "weekly" | "fortnightly" => CadenceUnit::Week,
        "month" | "monthly" => CadenceUnit::Month,
        "quarter" | "quarterly" => CadenceUnit::Quarter,
        "year" | "yearly" | "annual" | "annually" => CadenceUnit::Year,
        _ => return None,
    };
    let interval = if raw_unit == "fortnightly" { 2 } else { count };
    (interval > 0).then_some(RecurringCadence { interval, unit })
}

pub(super) fn unsupported_recurring_cadence(prompt: &str) -> Option<UnsupportedCadence> {
    if let Some(captures) = sub_minute_cadence_regex()?.captures(prompt) {
        return Some(UnsupportedCadence::SubMinute(captured_count(&captures)));
    }
    if ambiguous_cadence_regex().is_some_and(|regex| regex.is_match(prompt)) {
        return if prompt.contains("bimonthly") {
            Some(UnsupportedCadence::Bimonthly)
        } else {
            Some(UnsupportedCadence::Biweekly)
        };
    }
    vague_cadence_regex()
        .is_some_and(|regex| regex.is_match(prompt))
        .then_some(UnsupportedCadence::Vague)
}

pub(super) fn named_cadence_looks_like_noun(prompt: &str) -> bool {
    named_cadence_noun_regex().is_some_and(|regex| regex.is_match(prompt))
}

fn captured_count(captures: &regex::Captures<'_>) -> u32 {
    if captures.name("other").is_some() {
        return 2;
    }
    captures
        .name("count")
        .and_then(|value| cadence_count(value.as_str()))
        .unwrap_or(1)
}

fn cadence_count(raw: &str) -> Option<u32> {
    raw.parse().ok().or_else(|| {
        Some(match raw {
            "one" => 1,
            "two" => 2,
            "three" => 3,
            "four" => 4,
            "five" => 5,
            "six" => 6,
            "seven" => 7,
            "eight" => 8,
            "nine" => 9,
            "ten" => 10,
            "eleven" => 11,
            "twelve" => 12,
            _ => return None,
        })
    })
}

fn recurring_cadence_regex() -> Option<&'static Regex> {
    static REGEX: OnceLock<Result<Regex, regex::Error>> = OnceLock::new();
    REGEX
        .get_or_init(|| {
            Regex::new(
                r"(?ix)\b(?:
                    every\s+(?:(?:single\s+)|(?P<other>other)\s+|(?:(?P<count>\d+|one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve)\s+))?(?P<every_unit>minutes?|mins?|hours?|hrs?|days?|weeks?|months?|quarters?|years?)
                    |once\s+(?:a|an|per)\s+(?P<once_unit>minute|min|hour|hr|day|week|month|quarter|year)
                    |(?P<named>minutely|hourly|daily|weekly|fortnightly|monthly|quarterly|yearly|annual|annually)
                )\b",
            )
        })
        .as_ref()
        .ok()
}

fn sub_minute_cadence_regex() -> Option<&'static Regex> {
    static REGEX: OnceLock<Result<Regex, regex::Error>> = OnceLock::new();
    REGEX
        .get_or_init(|| {
            Regex::new(
                r"(?ix)\b(?:
                    every\s+(?:(?:single\s+)|(?P<other>other)\s+|(?:(?P<count>\d+|one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve)\s+))?(?:seconds?|secs?)
                    |once\s+(?:a|per)\s+(?:second|sec)
                )\b",
            )
        })
        .as_ref()
        .ok()
}

fn vague_cadence_regex() -> Option<&'static Regex> {
    static REGEX: OnceLock<Result<Regex, regex::Error>> = OnceLock::new();
    REGEX
        .get_or_init(|| {
            Regex::new(
                r"(?ix)\b(?:periodically|regularly|frequently|occasionally|from\s+time\s+to\s+time)\b",
            )
        })
        .as_ref()
        .ok()
}

fn ambiguous_cadence_regex() -> Option<&'static Regex> {
    static REGEX: OnceLock<Result<Regex, regex::Error>> = OnceLock::new();
    REGEX
        .get_or_init(|| Regex::new(r"(?i)\b(?:biweekly|bimonthly)\b"))
        .as_ref()
        .ok()
}

fn named_cadence_noun_regex() -> Option<&'static Regex> {
    static REGEX: OnceLock<Result<Regex, regex::Error>> = OnceLock::new();
    REGEX
        .get_or_init(|| Regex::new(r"(?i)\b(?:minutely|hourly|daily|weekly|biweekly|bimonthly|fortnightly|monthly|quarterly|yearly|annual|nightly)\s+(?:rate|rates|wage|wages|pay|price|prices|report|newsletter|meeting|statement|bill|brief|program\s+update|update|summary|plan|document|presentation)\b"))
        .as_ref()
        .ok()
}
