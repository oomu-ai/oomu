use chrono::{
    DateTime, Datelike, Duration as ChronoDuration, Local, NaiveDate, NaiveDateTime, TimeZone,
    Timelike, Utc, Weekday,
};
use chrono_tz::Tz;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IntervalUnit {
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Quarter,
    Year,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IntervalSchedule {
    amount: i64,
    unit: IntervalUnit,
}

pub(crate) fn next_run_after(expression: &str, after_ms: i64) -> Result<i64, String> {
    let expression = expression.trim();
    if expression.is_empty() {
        return Err("Schedule expression must not be empty.".to_string());
    }

    let normalized = expression.to_ascii_lowercase();
    if is_manual_expression(&normalized) {
        return Err("Manual run is not a recurring schedule.".to_string());
    }

    if normalized == "hourly" {
        return Ok(after_ms.saturating_add(60 * 60 * 1000));
    }

    if normalized == "daily" {
        return next_daily_run_after(9, 0, after_ms);
    }

    if let Some(rest) = normalized.strip_prefix("every ") {
        return next_interval_run_after(rest, after_ms);
    }

    if let Some(rest) = normalized.strip_prefix("daily at ") {
        let (hour, minute) = parse_daily_time(rest)?;
        return next_daily_run_after(hour, minute, after_ms);
    }

    let fields = normalized.split_whitespace().collect::<Vec<_>>();
    if fields.len() == 5 {
        return next_cron_run_after(&fields, after_ms);
    }

    Err(format!(
        "Unsupported workflow schedule expression: {expression}. Use examples like 'every 1 hour', 'daily at 09:00', or a simple five-field cron expression."
    ))
}

pub(crate) fn next_run_after_in_timezone(
    expression: &str,
    timezone: &str,
    after_ms: i64,
) -> Result<i64, String> {
    let expression = expression.trim().to_ascii_lowercase();
    if let Some(raw) = expression.strip_prefix("once:") {
        let instant = raw
            .parse::<i64>()
            .map_err(|_| "Invalid one-time schedule.".to_string())?;
        return (instant > after_ms)
            .then_some(instant)
            .ok_or_else(|| "The one-time schedule is already in the past.".to_string());
    }
    let zone: Tz = timezone
        .parse()
        .map_err(|_| "Invalid routine timezone.".to_string())?;
    if expression == "hourly" {
        return Ok(after_ms.saturating_add(60 * 60 * 1000));
    }
    if let Some(rest) = expression.strip_prefix("every ") {
        return next_interval_run_after_in_timezone(rest, zone, after_ms);
    }
    let after = Utc
        .timestamp_millis_opt(after_ms)
        .single()
        .ok_or_else(|| "Invalid schedule reference time.".to_string())?;
    if expression == "daily" || expression.starts_with("daily at ") {
        let (hour, minute) = if expression == "daily" {
            (9, 0)
        } else {
            parse_daily_time(expression.trim_start_matches("daily at "))?
        };
        return next_daily_run_after_in_timezone(zone, after, after_ms, hour, minute);
    }
    let fields = expression.split_whitespace().collect::<Vec<_>>();
    if fields.len() == 5 {
        return next_cron_run_after_in_timezone(&fields, zone, after);
    }
    next_run_after(&expression, after_ms)
}

fn next_daily_run_after_in_timezone(
    zone: Tz,
    after: DateTime<Utc>,
    after_ms: i64,
    hour: u32,
    minute: u32,
) -> Result<i64, String> {
    let local = after.with_timezone(&zone);
    for days in 0..=2 {
        let naive = (local.date_naive() + ChronoDuration::days(days))
            .and_hms_opt(hour, minute, 0)
            .ok_or_else(|| "Invalid daily schedule time.".to_string())?;
        if let Some(candidate) = resolve_daily_timezone_candidate(zone, naive, after_ms) {
            return Ok(candidate);
        }
    }
    Err("Unable to find the next daily run in this timezone.".to_string())
}

fn resolve_daily_timezone_candidate(zone: Tz, local: NaiveDateTime, after_ms: i64) -> Option<i64> {
    match zone.from_local_datetime(&local) {
        chrono::LocalResult::Single(value) => {
            (value.timestamp_millis() > after_ms).then_some(value.timestamp_millis())
        }
        chrono::LocalResult::Ambiguous(first, _) => {
            (first.timestamp_millis() > after_ms).then_some(first.timestamp_millis())
        }
        chrono::LocalResult::None => (1..=120).find_map(|offset| {
            let shifted = local + ChronoDuration::minutes(offset);
            match zone.from_local_datetime(&shifted) {
                chrono::LocalResult::Single(value) if value.timestamp_millis() > after_ms => {
                    Some(value.timestamp_millis())
                }
                _ => None,
            }
        }),
    }
}

fn next_cron_run_after_in_timezone(
    fields: &[&str],
    zone: Tz,
    after: DateTime<Utc>,
) -> Result<i64, String> {
    let mut candidate = (after + ChronoDuration::minutes(1))
        .with_second(0)
        .and_then(|value| value.with_nanosecond(0))
        .ok_or_else(|| "Invalid cron reference time.".to_string())?;
    for _ in 0..(366 * 24 * 60) {
        let local = candidate.with_timezone(&zone);
        if routine_cron_matches(
            fields,
            local.minute(),
            local.hour(),
            local.day(),
            local.month(),
            local.weekday().num_days_from_sunday(),
        )? && is_first_timezone_occurrence(zone, local)
        {
            return Ok(candidate.timestamp_millis());
        }
        candidate += ChronoDuration::minutes(1);
    }
    Err("Cron schedule has no run in the next year.".to_string())
}

fn is_first_timezone_occurrence(zone: Tz, local: DateTime<Tz>) -> bool {
    match zone.from_local_datetime(&local.naive_local()) {
        chrono::LocalResult::Ambiguous(first, _) => {
            first.timestamp_millis() == local.timestamp_millis()
        }
        _ => true,
    }
}

fn routine_cron_matches(
    fields: &[&str],
    minute: u32,
    hour: u32,
    day: u32,
    month: u32,
    weekday: u32,
) -> Result<bool, String> {
    Ok(routine_cron_field(fields[0], minute, 0, 59)?
        && routine_cron_field(fields[1], hour, 0, 23)?
        && routine_cron_field(fields[2], day, 1, 31)?
        && routine_cron_field(fields[3], month, 1, 12)?
        && routine_cron_field(fields[4], weekday, 0, 6)?)
}

fn routine_cron_field(raw: &str, value: u32, min: u32, max: u32) -> Result<bool, String> {
    if raw == "*" {
        return Ok(true);
    }
    if let Some(step) = raw.strip_prefix("*/") {
        let step = step
            .parse::<u32>()
            .map_err(|_| "Invalid cron step.".to_string())?;
        return Ok(step > 0 && (value - min) % step == 0);
    }
    raw.split(',').try_fold(false, |matched, item| {
        let parsed = item
            .parse::<u32>()
            .map_err(|_| "Invalid cron field.".to_string())?;
        if parsed < min || parsed > max {
            return Err("Cron value is out of range.".to_string());
        }
        Ok(matched || parsed == value)
    })
}

pub(crate) fn is_manual_expression(expression: &str) -> bool {
    matches!(expression.trim(), "manual" | "manual run")
}

fn next_interval_run_after(rest: &str, after_ms: i64) -> Result<i64, String> {
    let schedule = parse_interval_schedule(rest)?;
    match schedule.unit {
        IntervalUnit::Minute => {
            Ok(after_ms.saturating_add(schedule.amount.saturating_mul(60 * 1000)))
        }
        IntervalUnit::Hour => {
            Ok(after_ms.saturating_add(schedule.amount.saturating_mul(60 * 60 * 1000)))
        }
        IntervalUnit::Day => next_calendar_interval_run_after_local(
            after_ms,
            schedule.amount,
            CalendarInterval::Days(1),
        ),
        IntervalUnit::Week => next_calendar_interval_run_after_local(
            after_ms,
            schedule.amount,
            CalendarInterval::Days(7),
        ),
        IntervalUnit::Month => next_calendar_interval_run_after_local(
            after_ms,
            schedule.amount,
            CalendarInterval::Months(1),
        ),
        IntervalUnit::Quarter => next_calendar_interval_run_after_local(
            after_ms,
            schedule.amount,
            CalendarInterval::Months(3),
        ),
        IntervalUnit::Year => next_calendar_interval_run_after_local(
            after_ms,
            schedule.amount,
            CalendarInterval::Months(12),
        ),
    }
}

fn next_interval_run_after_in_timezone(rest: &str, zone: Tz, after_ms: i64) -> Result<i64, String> {
    let schedule = parse_interval_schedule(rest)?;
    match schedule.unit {
        IntervalUnit::Minute => {
            Ok(after_ms.saturating_add(schedule.amount.saturating_mul(60 * 1000)))
        }
        IntervalUnit::Hour => {
            Ok(after_ms.saturating_add(schedule.amount.saturating_mul(60 * 60 * 1000)))
        }
        IntervalUnit::Day => next_calendar_interval_run_after_in_timezone(
            after_ms,
            zone,
            schedule.amount,
            CalendarInterval::Days(1),
        ),
        IntervalUnit::Week => next_calendar_interval_run_after_in_timezone(
            after_ms,
            zone,
            schedule.amount,
            CalendarInterval::Days(7),
        ),
        IntervalUnit::Month => next_calendar_interval_run_after_in_timezone(
            after_ms,
            zone,
            schedule.amount,
            CalendarInterval::Months(1),
        ),
        IntervalUnit::Quarter => next_calendar_interval_run_after_in_timezone(
            after_ms,
            zone,
            schedule.amount,
            CalendarInterval::Months(3),
        ),
        IntervalUnit::Year => next_calendar_interval_run_after_in_timezone(
            after_ms,
            zone,
            schedule.amount,
            CalendarInterval::Months(12),
        ),
    }
}

fn parse_interval_schedule(rest: &str) -> Result<IntervalSchedule, String> {
    let parts = rest.split_whitespace().collect::<Vec<_>>();
    if parts.is_empty() || parts.len() > 2 {
        return Err(
            "Interval schedules must use 'every' followed by an optional number and one time unit."
                .to_string(),
        );
    }
    let (amount, raw_unit) = match parts.as_slice() {
        [unit] => (1, *unit),
        [amount, unit] => (
            amount
                .parse::<i64>()
                .map_err(|_| "Interval schedule amount must be a number.".to_string())?,
            *unit,
        ),
        _ => unreachable!(),
    };
    if amount <= 0 {
        return Err("Interval schedule amount must be greater than zero.".to_string());
    }
    let unit = match raw_unit.trim_end_matches('s') {
        "minute" | "min" => IntervalUnit::Minute,
        "hour" | "hr" => IntervalUnit::Hour,
        "day" => IntervalUnit::Day,
        "week" => IntervalUnit::Week,
        "month" => IntervalUnit::Month,
        "quarter" => IntervalUnit::Quarter,
        "year" => IntervalUnit::Year,
        _ => {
            return Err(format!(
                "Unsupported interval schedule unit: {raw_unit}. Use minutes, hours, days, weeks, months, quarters, or years."
            ))
        }
    };
    Ok(IntervalSchedule { amount, unit })
}

#[derive(Clone, Copy)]
enum CalendarInterval {
    Days(i64),
    Months(u32),
}

fn checked_interval_amount(amount: i64, multiplier: i64) -> Result<i64, String> {
    amount
        .checked_mul(multiplier)
        .filter(|value| *value > 0)
        .ok_or_else(|| "Interval schedule amount is too large.".to_string())
}

fn shifted_calendar_local(
    local: NaiveDateTime,
    amount: i64,
    interval: CalendarInterval,
) -> Result<NaiveDateTime, String> {
    match interval {
        CalendarInterval::Days(days) => local
            .checked_add_signed(ChronoDuration::days(checked_interval_amount(amount, days)?))
            .ok_or_else(|| "Interval schedule date is out of range.".to_string()),
        CalendarInterval::Months(months) => {
            let total_months = checked_interval_amount(amount, i64::from(months))?;
            add_months_clamped(local, total_months)
        }
    }
}

fn add_months_clamped(local: NaiveDateTime, months: i64) -> Result<NaiveDateTime, String> {
    let month_index = i64::from(local.year())
        .checked_mul(12)
        .and_then(|value| value.checked_add(i64::from(local.month0())))
        .and_then(|value| value.checked_add(months))
        .ok_or_else(|| "Interval schedule date is out of range.".to_string())?;
    let year = i32::try_from(month_index.div_euclid(12))
        .map_err(|_| "Interval schedule date is out of range.".to_string())?;
    let month = u32::try_from(month_index.rem_euclid(12) + 1)
        .map_err(|_| "Interval schedule date is out of range.".to_string())?;
    let first_next_month = if month == 12 {
        NaiveDate::from_ymd_opt(
            year.checked_add(1)
                .ok_or_else(|| "Interval schedule date is out of range.".to_string())?,
            1,
            1,
        )
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)
    }
    .ok_or_else(|| "Interval schedule date is out of range.".to_string())?;
    let last_day = (first_next_month - ChronoDuration::days(1)).day();
    let date = NaiveDate::from_ymd_opt(year, month, local.day().min(last_day))
        .ok_or_else(|| "Interval schedule date is out of range.".to_string())?;
    Ok(date.and_time(local.time()))
}

fn next_calendar_interval_run_after_in_timezone(
    after_ms: i64,
    zone: Tz,
    amount: i64,
    interval: CalendarInterval,
) -> Result<i64, String> {
    let after = Utc
        .timestamp_millis_opt(after_ms)
        .single()
        .ok_or_else(|| "Invalid schedule reference time.".to_string())?;
    let shifted =
        shifted_calendar_local(after.with_timezone(&zone).naive_local(), amount, interval)?;
    resolve_timezone_local_after(zone, shifted, after_ms)
}

fn resolve_timezone_local_after(
    zone: Tz,
    local: NaiveDateTime,
    after_ms: i64,
) -> Result<i64, String> {
    for offset in 0..=120 {
        let candidate = local + ChronoDuration::minutes(offset);
        match zone.from_local_datetime(&candidate) {
            chrono::LocalResult::Single(value) if value.timestamp_millis() > after_ms => {
                return Ok(value.timestamp_millis())
            }
            chrono::LocalResult::Ambiguous(first, second) => {
                if let Some(value) = [first, second]
                    .into_iter()
                    .find(|value| value.timestamp_millis() > after_ms)
                {
                    return Ok(value.timestamp_millis());
                }
            }
            _ => {}
        }
    }
    Err("Unable to resolve the next interval run in this timezone.".to_string())
}

fn next_calendar_interval_run_after_local(
    after_ms: i64,
    amount: i64,
    interval: CalendarInterval,
) -> Result<i64, String> {
    let after = local_datetime(after_ms)?;
    let shifted = shifted_calendar_local(after.naive_local(), amount, interval)?;
    for offset in 0..=120 {
        let candidate = shifted + ChronoDuration::minutes(offset);
        match Local.from_local_datetime(&candidate) {
            chrono::LocalResult::Single(value) if value.timestamp_millis() > after_ms => {
                return Ok(value.timestamp_millis())
            }
            chrono::LocalResult::Ambiguous(first, second) => {
                if let Some(value) = [first, second]
                    .into_iter()
                    .find(|value| value.timestamp_millis() > after_ms)
                {
                    return Ok(value.timestamp_millis());
                }
            }
            _ => {}
        }
    }
    Err("Unable to resolve the next local interval run.".to_string())
}

fn parse_daily_time(value: &str) -> Result<(u32, u32), String> {
    let (hour, minute) = value
        .trim()
        .split_once(':')
        .ok_or_else(|| "Daily schedules must use HH:MM time.".to_string())?;
    let hour = hour
        .parse::<u32>()
        .map_err(|_| "Daily schedule hour must be numeric.".to_string())?;
    let minute = minute
        .parse::<u32>()
        .map_err(|_| "Daily schedule minute must be numeric.".to_string())?;
    if hour > 23 || minute > 59 {
        return Err("Daily schedule time must be between 00:00 and 23:59.".to_string());
    }
    Ok((hour, minute))
}

fn next_daily_run_after(hour: u32, minute: u32, after_ms: i64) -> Result<i64, String> {
    let after = local_datetime(after_ms)?;
    for day_offset in 0..370 {
        let date = after.date_naive() + ChronoDuration::days(day_offset);
        let Some(naive) = date.and_hms_opt(hour, minute, 0) else {
            continue;
        };
        if let Some(candidate) = Local.from_local_datetime(&naive).earliest() {
            let candidate_ms = candidate.timestamp_millis();
            if candidate_ms > after_ms {
                return Ok(candidate_ms);
            }
        }
    }
    Err("Could not resolve the next local daily schedule time.".to_string())
}

fn next_cron_run_after(fields: &[&str], after_ms: i64) -> Result<i64, String> {
    let mut candidate = local_datetime(after_ms)? + ChronoDuration::minutes(1);
    candidate = candidate
        .with_second(0)
        .and_then(|value| value.with_nanosecond(0))
        .ok_or_else(|| "Could not align cron schedule to the next minute.".to_string())?;

    for _ in 0..(366 * 24 * 60) {
        if cron_matches(fields, candidate)? {
            return Ok(candidate.timestamp_millis());
        }
        candidate += ChronoDuration::minutes(1);
    }
    Err("Cron schedule did not match within one year.".to_string())
}

fn cron_matches(fields: &[&str], candidate: DateTime<Local>) -> Result<bool, String> {
    Ok(
        cron_field_matches(fields[0], candidate.minute(), 0, 59, false)?
            && cron_field_matches(fields[1], candidate.hour(), 0, 23, false)?
            && cron_field_matches(fields[2], candidate.day(), 1, 31, false)?
            && cron_field_matches(fields[3], candidate.month(), 1, 12, false)?
            && cron_field_matches(
                fields[4],
                weekday_from_sunday(candidate.weekday()),
                0,
                7,
                true,
            )?,
    )
}

fn cron_field_matches(
    field: &str,
    value: u32,
    min: u32,
    max: u32,
    sunday_alias: bool,
) -> Result<bool, String> {
    for term in field.split(',') {
        if cron_term_matches(term.trim(), value, min, max, sunday_alias)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn cron_term_matches(
    term: &str,
    value: u32,
    min: u32,
    max: u32,
    sunday_alias: bool,
) -> Result<bool, String> {
    if term.is_empty() {
        return Err("Cron fields must not contain empty terms.".to_string());
    }
    let (base, step) = if let Some((base, step)) = term.split_once('/') {
        let step = step
            .parse::<u32>()
            .map_err(|_| format!("Invalid cron step: {term}"))?;
        if step == 0 {
            return Err("Cron step must be greater than zero.".to_string());
        }
        (base, Some(step))
    } else {
        (term, None)
    };

    if base == "*" {
        return Ok(step
            .map(|step| value.saturating_sub(min) % step == 0)
            .unwrap_or(true));
    }

    if let Some((start, end)) = base.split_once('-') {
        let start = parse_cron_number(start, min, max, sunday_alias)?;
        let end = parse_cron_number(end, min, max, sunday_alias)?;
        if start > end {
            return Err(format!("Cron ranges must ascend: {term}"));
        }
        return Ok(value >= start
            && value <= end
            && step
                .map(|step| value.saturating_sub(start) % step == 0)
                .unwrap_or(true));
    }

    let start = parse_cron_number(base, min, max, sunday_alias)?;
    Ok(if let Some(step) = step {
        value >= start && value.saturating_sub(start) % step == 0
    } else {
        value == start
    })
}

fn parse_cron_number(raw: &str, min: u32, max: u32, sunday_alias: bool) -> Result<u32, String> {
    let value = raw
        .parse::<u32>()
        .map_err(|_| format!("Invalid cron number: {raw}"))?;
    let value = if sunday_alias && value == 7 { 0 } else { value };
    if value < min || value > max || (!sunday_alias && value == 7 && max == 7) {
        return Err(format!("Cron number out of range: {raw}"));
    }
    Ok(value)
}

fn weekday_from_sunday(weekday: Weekday) -> u32 {
    weekday.num_days_from_sunday()
}

pub(crate) fn local_datetime(timestamp_ms: i64) -> Result<DateTime<Local>, String> {
    Local
        .timestamp_millis_opt(timestamp_ms)
        .single()
        .ok_or_else(|| "Timestamp could not be represented in local time.".to_string())
}
