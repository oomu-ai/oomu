use super::*;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConflictFreeSlot {
    pub(crate) start_date: String,
    pub(crate) end_date: String,
    pub(crate) time_zone: String,
    pub(crate) blocking_event_count: usize,
}

/// Reads every enabled Calendar and freezes the earliest exact slot before any
/// mutation approval is requested. Callers persist this receipt and bind every
/// later surface to the same RFC 3339 timestamps.
pub(crate) async fn find_next_weekday_conflict_free_slot() -> Result<ConflictFreeSlot, String> {
    let (window_start, window_end) = next_weekday_local_window(
        CONFLICT_FREE_WINDOW_START_LOCAL,
        CONFLICT_FREE_WINDOW_END_LOCAL,
    )?;
    let calendar = super::super::eventkit_calendar::read_calendar(CalendarReadRequest {
        calendar_name: String::new(),
        start_timestamp: window_start.timestamp_millis() as f64 / 1_000.0,
        end_timestamp: window_end.timestamp_millis() as f64 / 1_000.0,
    })
    .await
    .map_err(calendar_failure_error)?;
    if calendar.truncated {
        return Err(
            "Calendar returned too many events to freeze a conflict-free slot safely. (calendar_conflict_window_truncated)"
                .to_string(),
        );
    }
    let occupied = calendar_event_intervals(&calendar.events)?;
    let slot = earliest_non_overlapping_slot(
        window_start.timestamp_millis(),
        window_end.timestamp_millis(),
        Duration::minutes(CONFLICT_FREE_EVENT_DURATION_MINUTES).num_milliseconds(),
        &occupied,
    )
    .ok_or_else(|| {
        "There is no conflict-free 30-minute slot on the next weekday between 1:00 PM and 4:00 PM. (calendar_conflict_window_full)"
            .to_string()
    })?;
    let start = DateTime::<Utc>::from_timestamp_millis(slot.start_timestamp_millis)
        .ok_or_else(|| "The selected Calendar start time is invalid.".to_string())?
        .with_timezone(&Local)
        .fixed_offset();
    let end = DateTime::<Utc>::from_timestamp_millis(slot.end_timestamp_millis)
        .ok_or_else(|| "The selected Calendar end time is invalid.".to_string())?
        .with_timezone(&Local)
        .fixed_offset();
    Ok(ConflictFreeSlot {
        start_date: canonical_rfc3339(start),
        end_date: canonical_rfc3339(end),
        time_zone: calendar.window.time_zone,
        blocking_event_count: occupied.len(),
    })
}

async fn exact_slot_is_still_available(
    start_timestamp_millis: i64,
    end_timestamp_millis: i64,
) -> Result<bool, String> {
    let calendar = super::super::eventkit_calendar::read_calendar(CalendarReadRequest {
        calendar_name: String::new(),
        start_timestamp: start_timestamp_millis as f64 / 1_000.0,
        end_timestamp: end_timestamp_millis as f64 / 1_000.0,
    })
    .await
    .map_err(calendar_failure_error)?;
    if calendar.truncated {
        return Err(
            "Calendar returned too many events to recheck the approved slot safely. (calendar_conflict_window_truncated)"
                .to_string(),
        );
    }
    let slot = OccupiedInterval {
        start_timestamp_millis,
        end_timestamp_millis,
    };
    Ok(calendar
        .events
        .iter()
        .all(|event| !event_blocks_slot(event) || !event_overlaps_slot(event, slot)))
}

/// Executes an exact, receipt-bound Calendar mutation for a registered wrapper
/// tool. The frozen slot is rechecked immediately before EventKit is allowed to
/// create anything.
pub(crate) fn execute_exact_conflict_checked_registration<'a>(
    context: TaskToolExecutionContext<'a>,
    arguments: Value,
    operation: &'static str,
) -> TaskToolFuture<'a> {
    Box::pin(async move {
        let validated = validate_registration(arguments)?;
        let request =
            serde_json::from_value::<CreateSystemCalendarEventRequest>(validated.arguments)
                .map_err(|_| {
                    "create_system_calendar_event arguments do not match the registered schema."
                        .to_string()
                })?;
        let execution_id = context.execution_id.ok_or_else(|| {
            "Creating a Calendar event requires an active approved Task.".to_string()
        })?;
        require_agent_runtime_task(context.persistence, execution_id)?;
        let start = DateTime::parse_from_rfc3339(&request.start_date)
            .map_err(|_| "Validated Calendar start date was unavailable.".to_string())?;
        let end = DateTime::parse_from_rfc3339(&request.end_date)
            .map_err(|_| "Validated Calendar end date was unavailable.".to_string())?;
        if !exact_slot_is_still_available(start.timestamp_millis(), end.timestamp_millis()).await? {
            return Err(
                "The approved Calendar time became busy before the event was created. The agenda and its frozen proposal are still saved; no event was added. (calendar_conflict_window_changed)"
                    .to_string(),
            );
        }
        create_verified_calendar_event(
            CalendarCreateRequest {
                calendar_name: request.calendar_name,
                title: request.title,
                start_timestamp_millis: start.timestamp_millis(),
                end_timestamp_millis: end.timestamp_millis(),
                start_date: request.start_date,
                end_date: request.end_date,
                location: request.location,
                notes: request.notes,
                availability: request.availability,
            },
            operation,
        )
        .await
    })
}

pub(crate) async fn verify_exact_event_postcondition(
    arguments: Value,
    receipt_message: &str,
    originally_requested_calendar: &str,
) -> Result<Value, String> {
    let validated = validate_registration(arguments)?;
    let request = serde_json::from_value::<CreateSystemCalendarEventRequest>(validated.arguments)
        .map_err(|_| {
        "create_system_calendar_event arguments do not match the registered schema.".to_string()
    })?;
    if receipt_message.len() > MAX_CALENDAR_RECEIPT_BYTES {
        return Err("Calendar receipt is too large to verify safely.".to_string());
    }
    let receipt = serde_json::from_str::<ConflictFreeCalendarReceipt>(receipt_message)
        .map_err(|_| "Calendar receipt is invalid.".to_string())?;
    let start = DateTime::parse_from_rfc3339(&request.start_date)
        .map_err(|_| "Calendar request startDate is invalid.".to_string())?;
    let _end = DateTime::parse_from_rfc3339(&request.end_date)
        .map_err(|_| "Calendar request endDate is invalid.".to_string())?;
    let window_date = start.date_naive();
    let offset = *start.offset();
    let window_start = offset
        .from_local_datetime(
            &window_date
                .and_hms_opt(CONFLICT_FREE_WINDOW_START_HOUR, 0, 0)
                .ok_or_else(|| "Calendar postcondition window is invalid.".to_string())?,
        )
        .single()
        .ok_or_else(|| "Calendar postcondition window is ambiguous.".to_string())?;
    let window_end = offset
        .from_local_datetime(
            &window_date
                .and_hms_opt(CONFLICT_FREE_WINDOW_END_HOUR, 0, 0)
                .ok_or_else(|| "Calendar postcondition window is invalid.".to_string())?,
        )
        .single()
        .ok_or_else(|| "Calendar postcondition window is ambiguous.".to_string())?;
    let calendar = read_calendar_requiring_unique_target(
        CalendarReadRequest {
            calendar_name: String::new(),
            start_timestamp: window_start.timestamp_millis() as f64 / 1_000.0,
            end_timestamp: window_end.timestamp_millis() as f64 / 1_000.0,
        },
        request.calendar_name.clone(),
        request.availability,
    )
    .await
    .map_err(calendar_failure_error)?;
    if calendar.truncated {
        return Err(
            "Calendar returned too many events to verify the final event uniquely. (calendar_conflict_window_truncated)"
                .to_string(),
        );
    }
    let exact = calendar
        .events
        .iter()
        .filter(|event| {
            event.calendar == request.calendar_name
                && event.name == request.title
                && event.start_time == request.start_date
                && event.end_time == request.end_date
                && event.location == request.location
                && event.notes == request.notes
                && event.availability == request.availability
                && !event.is_all_day
        })
        .collect::<Vec<_>>();
    let [event] = exact.as_slice() else {
        return Err(
            "Calendar no longer contains exactly one event matching the approved recovery meeting. (calendar_postcondition_not_unique)"
                .to_string(),
        );
    };
    if !same_title_is_unique_across_test_calendars(
        &calendar.events,
        event,
        originally_requested_calendar,
    ) {
        return Err(
            "Calendar contains another event with the approved recovery title in the requested window. (calendar_postcondition_title_not_unique)"
                .to_string(),
        );
    }
    let receipt_matches = receipt.status == "completed"
        && receipt.verified
        && receipt.exists
        && receipt.event_id == event.event_id
        && receipt.calendar_name == event.calendar
        && receipt.title == event.name
        && receipt.start_date == event.start_time
        && receipt.end_date == event.end_time
        && receipt.location == event.location
        && receipt.notes_verified
        && receipt.notes_sha256 == crate::foundation::digest::sha256_hex(event.notes.as_bytes())
        && receipt.availability == event.availability;
    if !receipt_matches || !event_is_conflict_free(&calendar.events, event) {
        return Err(
            "Calendar's final event no longer matches its verified receipt or now conflicts with another event. (calendar_postcondition_receipt_mismatch)"
                .to_string(),
        );
    }
    Ok(json!({
        "verified": true,
        "exists": true,
        "exactMatchCount": 1,
        "conflictFree": true,
        "eventIdSha256": crate::foundation::digest::sha256_hex(event.event_id.as_bytes()),
        "calendarName": event.calendar,
        "title": event.name,
        "startDate": event.start_time,
        "endDate": event.end_time,
        "availability": event.availability,
        "notesSha256": receipt.notes_sha256,
    }))
}

fn same_title_is_unique_across_test_calendars(
    events: &[CalendarEvent],
    accepted: &CalendarEvent,
    originally_requested_calendar: &str,
) -> bool {
    let same_title = events
        .iter()
        .filter(|candidate| {
            (candidate.calendar == accepted.calendar
                || candidate.calendar == originally_requested_calendar)
                && candidate.name == accepted.name
        })
        .collect::<Vec<_>>();
    same_title.len() == 1 && same_title[0].event_id == accepted.event_id
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(
        id: &str,
        availability: CalendarEventAvailability,
        is_all_day: bool,
        declined: bool,
    ) -> CalendarEvent {
        CalendarEvent {
            event_id: id.to_string(),
            calendar: "OOMU Test".to_string(),
            name: "OOMU Release Readiness".to_string(),
            start_time: "2026-07-21T13:00:00-04:00".to_string(),
            end_time: "2026-07-21T13:30:00-04:00".to_string(),
            location: String::new(),
            notes: String::new(),
            availability,
            is_all_day,
            declined_by_current_user: declined,
            time_zone: "America/New_York".to_string(),
        }
    }

    #[test]
    fn scenario_two_blocking_semantics_are_exact() {
        assert!(event_blocks_slot(&event(
            "timed-free",
            CalendarEventAvailability::Free,
            false,
            false,
        )));
        assert!(!event_blocks_slot(&event(
            "declined",
            CalendarEventAvailability::Busy,
            false,
            true,
        )));
        assert!(event_blocks_slot(&event(
            "all-day-busy",
            CalendarEventAvailability::Busy,
            true,
            false,
        )));
        assert!(!event_blocks_slot(&event(
            "all-day-tentative",
            CalendarEventAvailability::Tentative,
            true,
            false,
        )));
    }

    #[test]
    fn same_title_collision_covers_both_test_calendars_and_nonexact_events() {
        let accepted = event(
            "accepted",
            CalendarEventAvailability::Tentative,
            false,
            false,
        );
        assert!(same_title_is_unique_across_test_calendars(
            std::slice::from_ref(&accepted),
            &accepted,
            "Initial Test",
        ));

        let mut other_time = event(
            "other-time",
            CalendarEventAvailability::Tentative,
            false,
            false,
        );
        other_time.start_time = "2026-07-21T14:00:00-04:00".to_string();
        other_time.end_time = "2026-07-21T14:30:00-04:00".to_string();
        assert!(!same_title_is_unique_across_test_calendars(
            &[accepted.clone(), other_time],
            &accepted,
            "Initial Test",
        ));

        let mut denied_target = event(
            "denied-target",
            CalendarEventAvailability::Tentative,
            false,
            false,
        );
        denied_target.calendar = "Initial Test".to_string();
        denied_target.notes = "Different notes".to_string();
        assert!(!same_title_is_unique_across_test_calendars(
            &[accepted.clone(), denied_target],
            &accepted,
            "Initial Test",
        ));

        let mut unrelated = accepted.clone();
        unrelated.event_id = "unrelated".to_string();
        unrelated.calendar = "Personal".to_string();
        assert!(same_title_is_unique_across_test_calendars(
            &[accepted.clone(), unrelated],
            &accepted,
            "Initial Test",
        ));
    }
}
