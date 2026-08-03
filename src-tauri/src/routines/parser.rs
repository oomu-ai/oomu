use super::RoutineProposal;
use chrono::{Duration, NaiveDate, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use regex::Regex;

fn parse_time(raw: &str) -> Result<(u32, u32), String> {
    let normalized = raw.trim().to_ascii_lowercase();
    let captures = Regex::new(r"^(\d{1,2})(?::(\d{2}))?\s*(am|pm)?$")
        .unwrap()
        .captures(&normalized)
        .ok_or_else(|| "Use a time such as 9:00 AM or 17:30.".to_string())?;
    let mut hour = captures[1]
        .parse::<u32>()
        .map_err(|_| "Invalid hour.".to_string())?;
    let minute = captures
        .get(2)
        .map(|value| value.as_str().parse::<u32>())
        .transpose()
        .map_err(|_| "Invalid minute.".to_string())?
        .unwrap_or(0);
    match captures.get(3).map(|value| value.as_str()) {
        Some("pm") if hour < 12 => hour += 12,
        Some("am") if hour == 12 => hour = 0,
        Some(_) if hour > 12 => return Err("Invalid 12-hour time.".to_string()),
        _ => {}
    }
    if hour > 23 || minute > 59 {
        return Err("Invalid time.".to_string());
    }
    Ok((hour, minute))
}

pub(super) fn propose(
    text: &str,
    timezone: &str,
    after_ms: i64,
) -> Result<RoutineProposal, String> {
    let input = text.trim().to_ascii_lowercase();
    let _: Tz = timezone
        .parse()
        .map_err(|_| "Use a valid IANA timezone, such as America/New_York.".to_string())?;
    if matches!(input.as_str(), "biweekly" | "bimonthly") {
        return Err(format!(
            "'{input}' has more than one common meaning. Choose an explicit count and unit, such as 'every 2 weeks'."
        ));
    }
    let (expression, kind, summary) = if let Some(time) = input
        .strip_prefix("daily at ")
        .or_else(|| input.strip_prefix("every day at "))
    {
        let (hour, minute) = parse_time(time)?;
        (
            format!("daily at {hour:02}:{minute:02}"),
            "recurring",
            format!("Every day at {hour:02}:{minute:02} in {timezone}"),
        )
    } else if let Some(expression) = named_time_of_day_expression(&input) {
        (
            expression.to_string(),
            "recurring",
            format!("{} in {timezone}", recurring_summary(expression)),
        )
    } else if let Some(expression) = weekday_set_expression(&input, timezone, after_ms)? {
        (
            expression,
            "recurring",
            format!("{} in {timezone}", weekday_set_summary(&input)),
        )
    } else if let Some(expression) = canonical_interval_expression(&input) {
        (
            expression.clone(),
            "recurring",
            format!("{} in {timezone}", recurring_summary(&expression)),
        )
    } else if let Some(time) = input.strip_prefix("tomorrow at ") {
        let (hour, minute) = parse_time(time)?;
        let zone: Tz = timezone.parse().unwrap();
        let now = Utc
            .timestamp_millis_opt(after_ms)
            .single()
            .ok_or_else(|| "Invalid current time.".to_string())?
            .with_timezone(&zone);
        let date = now.date_naive() + Duration::days(1);
        let instant = local_instant(zone, date, hour, minute)?;
        let resolved = instant.with_timezone(&zone);
        (
            format!("once:{}", instant.timestamp_millis()),
            "one_shot",
            format!(
                "Once on {} at {:02}:{:02} in {timezone}",
                resolved.date_naive(),
                resolved.hour(),
                resolved.minute()
            ),
        )
    } else if let Some(captures) = Regex::new(r"^on (\d{4}-\d{2}-\d{2}) at (.+)$")
        .unwrap()
        .captures(&input)
    {
        let date = NaiveDate::parse_from_str(&captures[1], "%Y-%m-%d")
            .map_err(|_| "Use YYYY-MM-DD for a one-time date.".to_string())?;
        let (hour, minute) = parse_time(&captures[2])?;
        let zone: Tz = timezone.parse().unwrap();
        let instant = local_instant(zone, date, hour, minute)?;
        let resolved = instant.with_timezone(&zone);
        (
            format!("once:{}", instant.timestamp_millis()),
            "one_shot",
            format!(
                "Once on {} at {:02}:{:02} in {timezone}",
                resolved.date_naive(),
                resolved.hour(),
                resolved.minute()
            ),
        )
    } else if input.split_whitespace().count() == 5 {
        (
            input.clone(),
            "recurring",
            format!("Cron {input} in {timezone}"),
        )
    } else {
        return Err("Use a recurring interval from minutes through years, a weekday cadence, 'daily at 9 AM', 'tomorrow at 2 PM', a dated one-time run, or a five-field cron schedule.".to_string());
    };
    let mut cursor = after_ms;
    let mut next = vec![];
    for _ in 0..5 {
        cursor =
            crate::schedule_expression::next_run_after_in_timezone(&expression, timezone, cursor)?;
        next.push(cursor);
        if kind == "one_shot" {
            break;
        }
    }
    Ok(RoutineProposal {
        schedule_expression: expression,
        schedule_kind: kind.to_string(),
        timezone: timezone.to_string(),
        normalized_summary: summary,
        next_runs_ms: next,
    })
}

fn named_time_of_day_expression(input: &str) -> Option<&'static str> {
    match input {
        "every morning" | "each morning" | "every day in the morning" => Some("daily at 09:00"),
        "every afternoon" | "each afternoon" | "every day in the afternoon" => {
            Some("daily at 13:00")
        }
        "every evening" | "each evening" | "every day in the evening" => Some("daily at 18:00"),
        "nightly" | "every night" | "each night" => Some("daily at 21:00"),
        _ => None,
    }
}

fn recurring_summary(expression: &str) -> String {
    if let Some(time) = expression.strip_prefix("daily at ") {
        return format!("Every day at {time}");
    }
    let rest = expression.strip_prefix("every ").unwrap_or(expression);
    let parts = rest.split_whitespace().collect::<Vec<_>>();
    match parts.as_slice() {
        ["1", unit] => format!("Every {unit}"),
        [amount, unit] => format!("Every {amount} {unit}"),
        _ => format!("Every {rest}"),
    }
}

fn canonical_interval_expression(input: &str) -> Option<String> {
    let named = match input {
        "minutely" => Some("every 1 minute"),
        "hourly" => Some("every 1 hour"),
        "daily" => Some("every 1 day"),
        "weekly" => Some("every 1 week"),
        "fortnightly" => Some("every 2 weeks"),
        "monthly" => Some("every 1 month"),
        "quarterly" => Some("every 1 quarter"),
        "yearly" | "annually" => Some("every 1 year"),
        _ => None,
    };
    if let Some(expression) = named {
        return Some(expression.to_string());
    }

    let words = input.split_whitespace().collect::<Vec<_>>();
    let tail = match words.as_slice() {
        ["every" | "each", tail @ ..] => tail,
        ["once", "a" | "an" | "per", tail @ ..] => tail,
        _ => return None,
    };
    let (amount, raw_unit) = match tail {
        ["single", unit] => (1, *unit),
        ["other", unit] => (2, *unit),
        [amount, unit] => (interval_amount(amount)?, *unit),
        [unit] => (1, *unit),
        _ => return None,
    };
    let unit = canonical_interval_unit(raw_unit)?;
    let suffix = if amount == 1 {
        unit
    } else {
        plural_interval_unit(unit)
    };
    Some(format!("every {amount} {suffix}"))
}

fn interval_amount(raw: &str) -> Option<u32> {
    raw.parse::<u32>().ok().or_else(|| {
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

fn canonical_interval_unit(raw: &str) -> Option<&'static str> {
    match raw.trim_end_matches('s') {
        "second" | "sec" => Some("second"),
        "minute" | "min" => Some("minute"),
        "hour" | "hr" => Some("hour"),
        "day" => Some("day"),
        "week" => Some("week"),
        "month" => Some("month"),
        "quarter" => Some("quarter"),
        "year" => Some("year"),
        _ => None,
    }
}

fn plural_interval_unit(unit: &str) -> &str {
    match unit {
        "second" => "seconds",
        "minute" => "minutes",
        "hour" => "hours",
        "day" => "days",
        "week" => "weeks",
        "month" => "months",
        "quarter" => "quarters",
        "year" => "years",
        _ => unit,
    }
}

fn weekday_set_expression(
    input: &str,
    _timezone: &str,
    _after_ms: i64,
) -> Result<Option<String>, String> {
    let weekdays = match input {
        "every weekday" | "each weekday" | "weekdays" => Some("1,2,3,4,5"),
        "every weekend" | "each weekend" | "weekends" => Some("0,6"),
        "every sunday" | "sundays" => Some("0"),
        "every monday" | "mondays" => Some("1"),
        "every tuesday" | "tuesdays" => Some("2"),
        "every wednesday" | "wednesdays" => Some("3"),
        "every thursday" | "thursdays" => Some("4"),
        "every friday" | "fridays" => Some("5"),
        "every saturday" | "saturdays" => Some("6"),
        _ => None,
    };
    let Some(weekdays) = weekdays else {
        return Ok(None);
    };
    Ok(Some(format!("0 9 * * {weekdays}")))
}

fn weekday_set_summary(input: &str) -> String {
    let cadence = match input {
        "every weekday" | "each weekday" | "weekdays" => "Every weekday".to_string(),
        "every weekend" | "each weekend" | "weekends" => "Every weekend day".to_string(),
        value => format!(
            "Every {}",
            value.trim_start_matches("every ").trim_end_matches('s')
        ),
    };
    format!("{cadence} at 09:00")
}

fn local_instant(
    zone: Tz,
    date: NaiveDate,
    hour: u32,
    minute: u32,
) -> Result<chrono::DateTime<Tz>, String> {
    let naive = date
        .and_hms_opt(hour, minute, 0)
        .ok_or_else(|| "Invalid local time.".to_string())?;
    match zone.from_local_datetime(&naive) {
        chrono::LocalResult::Single(value) => Ok(value),
        chrono::LocalResult::Ambiguous(first, _) => Ok(first),
        chrono::LocalResult::None => {
            for offset in 1..=120 {
                let shifted = naive + Duration::minutes(offset);
                if let chrono::LocalResult::Single(value) = zone.from_local_datetime(&shifted) {
                    return Ok(value);
                }
            }
            Err("That local time does not exist because of a timezone transition.".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn daily_proposal_normalizes_timezone() {
        let proposal = propose("daily at 9 am", "America/New_York", 1_800_000_000_000).unwrap();
        assert_eq!(proposal.schedule_expression, "daily at 09:00");
        assert_eq!(proposal.next_runs_ms.len(), 5);
    }

    #[test]
    fn underspecified_weekly_proposal_uses_a_visible_timezone_local_default() {
        let now = Utc
            .with_ymd_and_hms(2026, 8, 2, 17, 25, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let proposal = propose("every week", "America/New_York", now).unwrap();
        assert_eq!(proposal.schedule_expression, "every 1 week");
        assert_eq!(proposal.schedule_kind, "recurring");
        assert_eq!(
            proposal.normalized_summary,
            "Every week in America/New_York"
        );
        assert_eq!(proposal.next_runs_ms.len(), 5);
        assert!(proposal.next_runs_ms[0] > now);
    }

    #[test]
    fn recurring_proposals_cover_every_supported_timeframe_through_one_parser() {
        let now = Utc
            .with_ymd_and_hms(2026, 8, 2, 17, 25, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let cases = [
            ("every 15 minutes", "every 15 minutes"),
            ("hourly", "every 1 hour"),
            ("every three days", "every 3 days"),
            ("fortnightly", "every 2 weeks"),
            ("monthly", "every 1 month"),
            ("every 2 quarters", "every 2 quarters"),
            ("annually", "every 1 year"),
            ("every other year", "every 2 years"),
        ];

        for (input, expected) in cases {
            let proposal = propose(input, "America/New_York", now).unwrap();
            assert_eq!(proposal.schedule_expression, expected, "{input}");
            assert_eq!(proposal.schedule_kind, "recurring", "{input}");
            assert_eq!(proposal.next_runs_ms.len(), 5, "{input}");
            assert!(proposal
                .next_runs_ms
                .windows(2)
                .all(|runs| runs[0] < runs[1]));
        }
    }

    #[test]
    fn recurring_proposals_cover_weekday_and_day_part_cadences_with_visible_defaults() {
        let now = Utc
            .with_ymd_and_hms(2026, 8, 2, 17, 25, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let weekdays = propose("every weekday", "America/New_York", now).unwrap();
        assert_eq!(weekdays.schedule_expression, "0 9 * * 1,2,3,4,5");
        assert_eq!(
            weekdays.normalized_summary,
            "Every weekday at 09:00 in America/New_York"
        );

        let nightly = propose("nightly", "America/New_York", now).unwrap();
        assert_eq!(nightly.schedule_expression, "daily at 21:00");
        assert_eq!(
            nightly.normalized_summary,
            "Every day at 21:00 in America/New_York"
        );
    }

    #[test]
    fn sub_minute_and_vague_cadences_fail_with_truthful_review_errors() {
        let seconds = propose("every second", "UTC", 1_800_000_000_000).unwrap_err();
        assert!(seconds.contains("Use minutes, hours, days, weeks, months, quarters, or years"));
        let vague = propose("periodically", "UTC", 1_800_000_000_000).unwrap_err();
        assert!(vague.contains("Use a recurring interval from minutes through years"));
        for ambiguous in ["biweekly", "bimonthly"] {
            let error = propose(ambiguous, "UTC", 1_800_000_000_000).unwrap_err();
            assert!(
                error.contains("more than one common meaning"),
                "{ambiguous}"
            );
        }
    }

    #[test]
    fn dated_proposal_accepts_same_day_future_and_rejects_past() {
        let now = Utc
            .with_ymd_and_hms(2026, 7, 21, 10, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let proposal =
            propose("on 2026-07-21 at 10:08", "UTC", now).expect("a same-day future run is valid");
        assert_eq!(proposal.schedule_kind, "one_shot");
        assert_eq!(proposal.next_runs_ms, vec![now + 8 * 60 * 1_000]);
        let error = propose("on 2026-07-21 at 09:59", "UTC", now)
            .expect_err("a past one-time run is rejected");
        assert_eq!(error, "The one-time schedule is already in the past.");
    }

    #[test]
    fn tomorrow_and_later_one_time_schedules_remain_valid() {
        let now = Utc
            .with_ymd_and_hms(2026, 7, 21, 23, 55, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let tomorrow = propose("tomorrow at 12:05 am", "UTC", now)
            .expect("tomorrow remains a supported one-time schedule");
        assert_eq!(tomorrow.schedule_kind, "one_shot");
        assert_eq!(tomorrow.next_runs_ms, vec![now + 10 * 60 * 1_000]);

        let later = propose("on 2026-07-24 at 14:30", "UTC", now)
            .expect("future dated one-time schedules remain supported");
        assert_eq!(later.schedule_kind, "one_shot");
        assert!(later.next_runs_ms[0] > tomorrow.next_runs_ms[0]);
    }

    #[test]
    fn one_time_proposal_uses_selected_timezone_instead_of_host_timezone() {
        let now = Utc
            .with_ymd_and_hms(2026, 7, 21, 0, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let proposal = propose("on 2026-07-20 at 15:00", "Pacific/Honolulu", now)
            .expect("the selected timezone makes this same-date wall time future");
        assert_eq!(
            proposal.next_runs_ms,
            vec![Utc
                .with_ymd_and_hms(2026, 7, 21, 1, 0, 0)
                .single()
                .unwrap()
                .timestamp_millis()]
        );
        assert_eq!(
            proposal.normalized_summary,
            "Once on 2026-07-20 at 15:00 in Pacific/Honolulu"
        );
    }

    #[test]
    fn one_time_proposal_normalizes_dst_gaps_and_chooses_first_overlap() {
        let spring_now = Utc
            .with_ymd_and_hms(2026, 3, 8, 5, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let spring = propose("on 2026-03-08 at 02:30", "America/New_York", spring_now)
            .expect("a nonexistent wall time advances to the first real minute");
        assert_eq!(
            spring.next_runs_ms,
            vec![Utc
                .with_ymd_and_hms(2026, 3, 8, 7, 0, 0)
                .single()
                .unwrap()
                .timestamp_millis()]
        );
        assert_eq!(
            spring.normalized_summary,
            "Once on 2026-03-08 at 03:00 in America/New_York"
        );

        let fall_now = Utc
            .with_ymd_and_hms(2026, 11, 1, 4, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let fall = propose("on 2026-11-01 at 01:30", "America/New_York", fall_now)
            .expect("an ambiguous wall time chooses the first occurrence");
        assert_eq!(
            fall.next_runs_ms,
            vec![Utc
                .with_ymd_and_hms(2026, 11, 1, 5, 30, 0)
                .single()
                .unwrap()
                .timestamp_millis()]
        );
    }
}
