use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveTime, TimeZone, Weekday};

pub(super) fn parse_local_time(field: &str, value: &str) -> Result<NaiveTime, String> {
    if value.len() != 5 || value.contains('\0') {
        return Err(format!(
            "create_conflict_free_calendar_event {field} must use 24-hour HH:MM local time."
        ));
    }
    NaiveTime::parse_from_str(value, "%H:%M").map_err(|_| {
        format!("create_conflict_free_calendar_event {field} must use 24-hour HH:MM local time.")
    })
}

pub(super) fn next_weekday_after(date: NaiveDate) -> Option<NaiveDate> {
    let mut candidate = date;
    for _ in 0..3 {
        candidate = candidate.checked_add_signed(Duration::days(1))?;
        if !matches!(candidate.weekday(), Weekday::Sat | Weekday::Sun) {
            return Some(candidate);
        }
    }
    None
}

pub(super) fn local_window(
    date: NaiveDate,
    window_start_local: &str,
    window_end_local: &str,
) -> Result<(DateTime<Local>, DateTime<Local>), String> {
    let start_time = parse_local_time("windowStartLocal", window_start_local)?;
    let end_time = parse_local_time("windowEndLocal", window_end_local)?;
    let start = Local
        .from_local_datetime(&date.and_time(start_time))
        .single()
        .ok_or_else(|| {
            "The next weekday Calendar window is ambiguous in local time.".to_string()
        })?;
    let end = Local
        .from_local_datetime(&date.and_time(end_time))
        .single()
        .ok_or_else(|| {
            "The next weekday Calendar window is ambiguous in local time.".to_string()
        })?;
    Ok((start, end))
}

pub(super) fn next_weekday_window(
    window_start_local: &str,
    window_end_local: &str,
) -> Result<(DateTime<Local>, DateTime<Local>), String> {
    let date = next_weekday_after(Local::now().date_naive()).ok_or_else(|| {
        "The next weekday Calendar window is outside the supported range.".to_string()
    })?;
    local_window(date, window_start_local, window_end_local)
}
