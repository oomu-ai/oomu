use super::{
    eventkit_calendar::{
        create_calendar_event, read_calendar_requiring_unique_target, remove_calendar_event,
        CalendarCreateRequest, CalendarCreateSuccess, CalendarEvent, CalendarEventAvailability,
        CalendarReadFailure, CalendarReadRequest,
    },
    task_runtime::require_agent_runtime_task,
    task_tool_runtime::{
        TaskToolApprovalTier, TaskToolExecutionContext, TaskToolFuture, TaskToolMetadata,
        TaskToolRegistration, TaskToolRiskTier, TaskToolValidation,
    },
};
use crate::shield_gate::{CommandStatus, ExecuteCommandResponse};
#[cfg(test)]
use chrono::NaiveDate;
use chrono::{DateTime, Duration, FixedOffset, Local, NaiveTime, SecondsFormat, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[path = "system_calendar_event/error.rs"]
mod error;
pub(crate) use error::calendar_failure_error;
#[path = "system_calendar_event/exact_conflict.rs"]
mod exact_conflict;
pub(crate) use exact_conflict::{
    execute_exact_conflict_checked_registration, find_next_weekday_conflict_free_slot,
    verify_exact_event_postcondition,
};
#[path = "system_calendar_event/window.rs"]
mod window;
#[cfg(test)]
use window::{local_window as local_conflict_free_window, next_weekday_after};
use window::{
    next_weekday_window as next_weekday_local_window,
    parse_local_time as parse_conflict_free_local_time,
};

const MAX_CALENDAR_NAME_CHARACTERS: usize = 512;
const MAX_TITLE_CHARACTERS: usize = 2_048;
const MAX_LOCATION_CHARACTERS: usize = 2_048;
const MAX_NOTES_CHARACTERS: usize = 16_384;
const MAX_RFC3339_CHARACTERS: usize = 64;
const MAX_EVENT_DURATION_MILLISECONDS: i64 = 24 * 60 * 60 * 1_000;
const EARLIEST_EVENT_TIMESTAMP_MILLISECONDS: i64 = 946_684_800_000; // 2000-01-01T00:00:00Z
const LATEST_EVENT_TIMESTAMP_MILLISECONDS: i64 = 4_102_444_800_000; // 2100-01-01T00:00:00Z
const CONFLICT_FREE_EVENT_DURATION_MINUTES: i64 = 30;
const CONFLICT_FREE_WINDOW_START_HOUR: u32 = 13;
const CONFLICT_FREE_WINDOW_END_HOUR: u32 = 16;
const CONFLICT_FREE_DAY: &str = "next_weekday";
const CONFLICT_FREE_WINDOW_START_LOCAL: &str = "13:00";
const CONFLICT_FREE_WINDOW_END_LOCAL: &str = "16:00";
const CONFLICT_FREE_EARLIEST_LOCAL: &str = "06:00";
const CONFLICT_FREE_LATEST_LOCAL: &str = "22:00";
const CONFLICT_FREE_MAX_WINDOW_MINUTES: i64 = 12 * 60;
const CONFLICT_FREE_COMMIT_ATTEMPTS: usize = 3;
const MAX_CALENDAR_RECEIPT_BYTES: usize = 32 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateSystemCalendarEventRequest {
    calendar_name: String,
    title: String,
    start_date: String,
    end_date: String,
    location: String,
    notes: String,
    availability: CalendarEventAvailability,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateConflictFreeCalendarEventRequest {
    calendar_name: String,
    title: String,
    day: String,
    window_start_local: String,
    window_end_local: String,
    duration_minutes: i64,
    location: String,
    notes: String,
    availability: CalendarEventAvailability,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConflictFreeCalendarReceipt {
    status: String,
    verified: bool,
    exists: bool,
    created: bool,
    event_id: String,
    calendar_name: String,
    title: String,
    start_date: String,
    end_date: String,
    location: String,
    notes_sha256: String,
    notes_verified: bool,
    availability: CalendarEventAvailability,
    reused_existing: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OccupiedInterval {
    start_timestamp_millis: i64,
    end_timestamp_millis: i64,
}

pub(crate) fn register_task_tool() -> Result<(), String> {
    crate::tools::task_tool_runtime::register(TaskToolRegistration {
        operation: "create_system_calendar_event",
        validate: validate_registration,
        validate_resolved: validate_registration,
        resolve: crate::tools::task_tool_runtime::identity_resolver,
        execute: execute_registration,
        planner_context: None,
        schema: create_system_calendar_event_schema,
        metadata: TaskToolMetadata {
            description: "Create one event in an exact named macOS Calendar after approval, then read it back and verify every scheduling field.",
            risk_tier: TaskToolRiskTier::SystemExec,
            approval_tier: TaskToolApprovalTier::Explicit,
            agent_error_code: "calendar_event_creation_failed",
            agent_error_boundary: "CreateSystemCalendarEvent",
            execution_path: "The native create_system_calendar_event tool saved the approved event in the exact named writable calendar and verified the saved EventKit record.",
        },
    })?;
    crate::tools::task_tool_runtime::register(TaskToolRegistration {
        operation: "create_conflict_free_calendar_event",
        validate: validate_conflict_free_registration,
        validate_resolved: validate_conflict_free_registration,
        resolve: crate::tools::task_tool_runtime::identity_resolver,
        execute: execute_conflict_free_registration,
        planner_context: None,
        schema: create_conflict_free_calendar_event_schema,
        metadata: TaskToolMetadata {
            description: "Read every macOS Calendar for conflicts in the exact next-weekday local window, create one tentative 30-minute event in the exact named writable calendar at the earliest available time after approval, then verify the saved EventKit record.",
            risk_tier: TaskToolRiskTier::SystemExec,
            approval_tier: TaskToolApprovalTier::Explicit,
            agent_error_code: "calendar_event_creation_failed",
            agent_error_boundary: "CreateConflictFreeCalendarEvent",
            execution_path: "The native create_conflict_free_calendar_event tool read all Calendar event calendars, chose the earliest verified conflict-free slot, saved the approved event in the exact named calendar, and verified the saved EventKit record.",
        },
    })
}

fn create_system_calendar_event_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "calendarName": {
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_CALENDAR_NAME_CHARACTERS
            },
            "title": {
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_TITLE_CHARACTERS
            },
            "startDate": {
                "type": "string",
                "format": "date-time",
                "minLength": 20,
                "maxLength": MAX_RFC3339_CHARACTERS
            },
            "endDate": {
                "type": "string",
                "format": "date-time",
                "minLength": 20,
                "maxLength": MAX_RFC3339_CHARACTERS
            },
            "location": {
                "type": "string",
                "maxLength": MAX_LOCATION_CHARACTERS
            },
            "notes": {
                "type": "string",
                "maxLength": MAX_NOTES_CHARACTERS
            },
            "availability": {
                "type": "string",
                "enum": ["busy", "free", "tentative"]
            }
        },
        "required": [
            "calendarName",
            "title",
            "startDate",
            "endDate",
            "location",
            "notes",
            "availability"
        ],
        "additionalProperties": false
    })
}

fn create_conflict_free_calendar_event_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "calendarName": {
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_CALENDAR_NAME_CHARACTERS
            },
            "title": {
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_TITLE_CHARACTERS
            },
            "day": {
                "type": "string",
                "enum": [CONFLICT_FREE_DAY]
            },
            "windowStartLocal": {
                "type": "string",
                "pattern": "^(?:0[6-9]|1[0-9]|2[0-1]):[0-5][0-9]$",
                "description": "Approved next-weekday local window start from 06:00 through 21:59."
            },
            "windowEndLocal": {
                "type": "string",
                "pattern": "^(?:0[6-9]|1[0-9]|2[0-2]):[0-5][0-9]$",
                "description": "Approved next-weekday local window end after the start and no later than 22:00."
            },
            "durationMinutes": {
                "type": "integer",
                "enum": [CONFLICT_FREE_EVENT_DURATION_MINUTES]
            },
            "location": {
                "type": "string",
                "maxLength": MAX_LOCATION_CHARACTERS
            },
            "notes": {
                "type": "string",
                "maxLength": MAX_NOTES_CHARACTERS
            },
            "availability": {
                "type": "string",
                "enum": ["tentative"]
            }
        },
        "required": [
            "calendarName",
            "title",
            "day",
            "windowStartLocal",
            "windowEndLocal",
            "durationMinutes",
            "location",
            "notes",
            "availability"
        ],
        "additionalProperties": false
    })
}

pub(crate) fn validate_registration(arguments: Value) -> Result<TaskToolValidation, String> {
    let mut request = serde_json::from_value::<CreateSystemCalendarEventRequest>(arguments)
        .map_err(|_| {
            "create_system_calendar_event arguments do not match the registered schema.".to_string()
        })?;
    request.calendar_name = request.calendar_name.trim().to_string();
    request.title = request.title.trim().to_string();
    request.start_date = request.start_date.trim().to_string();
    request.end_date = request.end_date.trim().to_string();
    request.location = request.location.trim().to_string();
    request.notes = request.notes.trim().to_string();

    validate_text(
        "calendarName",
        &request.calendar_name,
        1,
        MAX_CALENDAR_NAME_CHARACTERS,
    )?;
    validate_text("title", &request.title, 1, MAX_TITLE_CHARACTERS)?;
    validate_text("location", &request.location, 0, MAX_LOCATION_CHARACTERS)?;
    validate_text("notes", &request.notes, 0, MAX_NOTES_CHARACTERS)?;

    let start = parse_bounded_date("startDate", &request.start_date)?;
    let end = parse_bounded_date("endDate", &request.end_date)?;
    let start_millis = start.timestamp_millis();
    let end_millis = end.timestamp_millis();
    let duration_millis = end_millis.saturating_sub(start_millis);
    if duration_millis <= 0 || duration_millis > MAX_EVENT_DURATION_MILLISECONDS {
        return Err(
            "create_system_calendar_event requires startDate before endDate and a duration of at most 24 hours."
                .to_string(),
        );
    }

    request.start_date = canonical_rfc3339(start);
    request.end_date = canonical_rfc3339(end);
    Ok(TaskToolValidation {
        arguments: serde_json::to_value(request).map_err(|error| error.to_string())?,
        potentially_effectful: true,
    })
}

fn validate_conflict_free_registration(arguments: Value) -> Result<TaskToolValidation, String> {
    let mut request = serde_json::from_value::<CreateConflictFreeCalendarEventRequest>(arguments)
        .map_err(|_| {
        "create_conflict_free_calendar_event arguments do not match the registered schema."
            .to_string()
    })?;
    request.calendar_name = request.calendar_name.trim().to_string();
    request.title = request.title.trim().to_string();
    request.day = request.day.trim().to_string();
    request.window_start_local = request.window_start_local.trim().to_string();
    request.window_end_local = request.window_end_local.trim().to_string();
    request.location = request.location.trim().to_string();
    request.notes = request.notes.trim().to_string();
    validate_text(
        "calendarName",
        &request.calendar_name,
        1,
        MAX_CALENDAR_NAME_CHARACTERS,
    )?;
    validate_text("title", &request.title, 1, MAX_TITLE_CHARACTERS)?;
    validate_text("location", &request.location, 0, MAX_LOCATION_CHARACTERS)?;
    validate_text("notes", &request.notes, 0, MAX_NOTES_CHARACTERS)?;
    let start = parse_conflict_free_local_time("windowStartLocal", &request.window_start_local)?;
    let end = parse_conflict_free_local_time("windowEndLocal", &request.window_end_local)?;
    let earliest = NaiveTime::parse_from_str(CONFLICT_FREE_EARLIEST_LOCAL, "%H:%M")
        .expect("static earliest Calendar time");
    let latest = NaiveTime::parse_from_str(CONFLICT_FREE_LATEST_LOCAL, "%H:%M")
        .expect("static latest Calendar time");
    let window_minutes = end.signed_duration_since(start).num_minutes();
    if request.day != CONFLICT_FREE_DAY
        || start < earliest
        || end > latest
        || window_minutes < CONFLICT_FREE_EVENT_DURATION_MINUTES
        || window_minutes > CONFLICT_FREE_MAX_WINDOW_MINUTES
        || request.duration_minutes != CONFLICT_FREE_EVENT_DURATION_MINUTES
        || request.availability != CalendarEventAvailability::Tentative
    {
        return Err(
            "create_conflict_free_calendar_event requires next_weekday, a bounded 06:00-22:00 local window that fits one 30-minute event, a 30-minute duration, and tentative availability."
                .to_string(),
        );
    }
    Ok(TaskToolValidation {
        arguments: serde_json::to_value(request).map_err(|error| error.to_string())?,
        potentially_effectful: true,
    })
}

fn validate_text(
    field: &str,
    value: &str,
    minimum_characters: usize,
    maximum_characters: usize,
) -> Result<(), String> {
    let characters = value.chars().count();
    if value.contains('\0') || characters < minimum_characters || characters > maximum_characters {
        return Err(format!(
            "create_system_calendar_event {field} is outside the bounded contract."
        ));
    }
    Ok(())
}

fn parse_bounded_date(field: &str, value: &str) -> Result<DateTime<FixedOffset>, String> {
    if value.len() > MAX_RFC3339_CHARACTERS || value.contains('\0') {
        return Err(format!(
            "create_system_calendar_event {field} must be a bounded RFC 3339 timestamp."
        ));
    }
    let parsed = DateTime::parse_from_rfc3339(value).map_err(|_| {
        format!("create_system_calendar_event {field} must be an RFC 3339 timestamp.")
    })?;
    if parsed.timestamp_subsec_nanos() % 1_000_000 != 0 {
        return Err(format!(
            "create_system_calendar_event {field} supports millisecond precision."
        ));
    }
    if !(EARLIEST_EVENT_TIMESTAMP_MILLISECONDS..=LATEST_EVENT_TIMESTAMP_MILLISECONDS)
        .contains(&parsed.timestamp_millis())
    {
        return Err(format!(
            "create_system_calendar_event {field} must be between 2000 and 2100."
        ));
    }
    Ok(parsed)
}

fn canonical_rfc3339(value: DateTime<FixedOffset>) -> String {
    let precision = if value.timestamp_subsec_millis() == 0 {
        SecondsFormat::Secs
    } else {
        SecondsFormat::Millis
    };
    value.to_rfc3339_opts(precision, true)
}

fn earliest_non_overlapping_slot(
    window_start_millis: i64,
    window_end_millis: i64,
    duration_millis: i64,
    occupied: &[OccupiedInterval],
) -> Option<OccupiedInterval> {
    if duration_millis <= 0
        || window_end_millis
            .checked_sub(window_start_millis)
            .is_none_or(|window| window < duration_millis)
    {
        return None;
    }
    let mut occupied = occupied.to_vec();
    occupied.sort_by_key(|interval| {
        (
            interval.start_timestamp_millis,
            interval.end_timestamp_millis,
        )
    });
    let mut candidate_start = window_start_millis;
    for interval in occupied {
        if interval.end_timestamp_millis <= candidate_start
            || interval.start_timestamp_millis >= window_end_millis
        {
            continue;
        }
        let candidate_end = candidate_start.checked_add(duration_millis)?;
        if candidate_end <= interval.start_timestamp_millis {
            return Some(OccupiedInterval {
                start_timestamp_millis: candidate_start,
                end_timestamp_millis: candidate_end,
            });
        }
        candidate_start = candidate_start.max(interval.end_timestamp_millis);
    }
    let candidate_end = candidate_start.checked_add(duration_millis)?;
    (candidate_end <= window_end_millis).then_some(OccupiedInterval {
        start_timestamp_millis: candidate_start,
        end_timestamp_millis: candidate_end,
    })
}

fn calendar_event_intervals(events: &[CalendarEvent]) -> Result<Vec<OccupiedInterval>, String> {
    events
        .iter()
        .filter(|event| event_blocks_slot(event))
        .map(|event| {
            let start = DateTime::parse_from_rfc3339(&event.start_time)
                .map_err(|_| "Calendar returned an invalid event start time.".to_string())?
                .timestamp_millis();
            let end = DateTime::parse_from_rfc3339(&event.end_time)
                .map_err(|_| "Calendar returned an invalid event end time.".to_string())?
                .timestamp_millis();
            if end <= start {
                return Err("Calendar returned an event with an invalid duration.".to_string());
            }
            Ok(OccupiedInterval {
                start_timestamp_millis: start,
                end_timestamp_millis: end,
            })
        })
        .collect()
}

fn event_blocks_slot(event: &CalendarEvent) -> bool {
    !event.declined_by_current_user
        && (!event.is_all_day || event.availability == CalendarEventAvailability::Busy)
}

async fn create_conflict_free_calendar_event(
    request: CreateConflictFreeCalendarEventRequest,
) -> Result<ExecuteCommandResponse, String> {
    let (window_start, window_end) =
        next_weekday_local_window(&request.window_start_local, &request.window_end_local)?;
    let requested_window = OccupiedInterval {
        start_timestamp_millis: window_start.timestamp_millis(),
        end_timestamp_millis: window_end.timestamp_millis(),
    };
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
            "Calendar returned too many events to choose a conflict-free slot safely. (calendar_conflict_window_truncated)"
                .to_string(),
        );
    }
    let existing = calendar
        .events
        .iter()
        .filter(|event| existing_conflict_free_event_matches(event, &request, requested_window))
        .collect::<Vec<_>>();
    if existing.len() > 1 {
        return Err(format!(
            "More than one matching '{}' event already exists. OOMU will not create another; remove the duplicate and retry. (calendar_duplicate_existing_events)",
            request.title
        ));
    }
    if let Some(event) = existing.first() {
        if !event_is_conflict_free(&calendar.events, event) {
            return Err(format!(
                "The matching '{}' event currently conflicts with another Calendar event. Resolve the conflict and retry. (calendar_existing_event_conflicted)",
                request.title
            ));
        }
        return Ok(calendar_event_receipt(
            CalendarCreateSuccess {
                event_id: event.event_id.clone(),
                calendar_name: event.calendar.clone(),
                title: event.name.clone(),
                start_date: event.start_time.clone(),
                end_date: event.end_time.clone(),
                location: event.location.clone(),
                notes: event.notes.clone(),
                availability: event.availability,
            },
            "create_conflict_free_calendar_event",
            true,
        ));
    }
    let mut occupied = calendar_event_intervals(&calendar.events)?;
    let duration_millis =
        Duration::minutes(CONFLICT_FREE_EVENT_DURATION_MINUTES).num_milliseconds();
    for _ in 0..CONFLICT_FREE_COMMIT_ATTEMPTS {
        let slot = earliest_non_overlapping_slot(
            window_start.timestamp_millis(),
            window_end.timestamp_millis(),
            duration_millis,
            &occupied,
        )
        .ok_or_else(|| {
            format!(
                "The exact requested calendar has no conflict-free 30-minute slot on the next weekday between {} and {} local time. (calendar_conflict_window_full)",
                request.window_start_local, request.window_end_local
            )
        })?;
        let start = DateTime::<Utc>::from_timestamp_millis(slot.start_timestamp_millis)
            .ok_or_else(|| "The selected Calendar start time is invalid.".to_string())?
            .with_timezone(&Local)
            .fixed_offset();
        let end = DateTime::<Utc>::from_timestamp_millis(slot.end_timestamp_millis)
            .ok_or_else(|| "The selected Calendar end time is invalid.".to_string())?
            .with_timezone(&Local)
            .fixed_offset();
        let created = create_calendar_event(CalendarCreateRequest {
            calendar_name: request.calendar_name.clone(),
            title: request.title.clone(),
            start_timestamp_millis: slot.start_timestamp_millis,
            end_timestamp_millis: slot.end_timestamp_millis,
            start_date: canonical_rfc3339(start),
            end_date: canonical_rfc3339(end),
            location: request.location.clone(),
            notes: request.notes.clone(),
            availability: request.availability,
        })
        .await
        .map_err(calendar_failure_error)?;
        let post_create = match read_calendar_requiring_unique_target(
            CalendarReadRequest {
                calendar_name: String::new(),
                start_timestamp: window_start.timestamp_millis() as f64 / 1_000.0,
                end_timestamp: window_end.timestamp_millis() as f64 / 1_000.0,
            },
            request.calendar_name.clone(),
            request.availability,
        )
        .await
        {
            Ok(calendar) => calendar,
            Err(failure) => {
                remove_calendar_event(created.event_id.clone())
                    .await
                    .map_err(|cleanup_failure| {
                        format!(
                            "Calendar could not complete its second verification read and could not remove the event from this run. Review Calendar before retrying. Original error: {} ({}); cleanup error: {} ({})",
                            failure.message,
                            failure.code,
                            cleanup_failure.message,
                            cleanup_failure.code
                        )
                    })?;
                return Err(format!(
                    "Calendar could not complete its second verification read. The event from this run was removed. {} ({})",
                    failure.message, failure.code
                ));
            }
        };
        if post_create.truncated {
            remove_calendar_event(created.event_id.clone())
                .await
                .map_err(calendar_failure_error)?;
            return Err(
                "Calendar changed and returned too many events to verify the selected slot safely. The unverified event was removed. (calendar_conflict_window_truncated)"
                    .to_string(),
            );
        }
        let created_is_present = post_create.events.iter().any(|event| {
            event.event_id == created.event_id
                && event.calendar == created.calendar_name
                && event.name == created.title
                && DateTime::parse_from_rfc3339(&event.start_time)
                    .is_ok_and(|value| value.timestamp_millis() == slot.start_timestamp_millis)
                && DateTime::parse_from_rfc3339(&event.end_time)
                    .is_ok_and(|value| value.timestamp_millis() == slot.end_timestamp_millis)
                && event.location == created.location
                && event.notes == created.notes
                && event.availability == created.availability
                && !event.is_all_day
        });
        let exact_matches = post_create
            .events
            .iter()
            .filter(|event| existing_conflict_free_event_matches(event, &request, requested_window))
            .collect::<Vec<_>>();
        if exact_matches.len() > 1 {
            remove_calendar_event(created.event_id.clone())
                .await
                .map_err(calendar_failure_error)?;
            return Err(
                "More than one matching event appeared while Calendar was being updated. The event from this run was removed. (calendar_duplicate_existing_events)"
                    .to_string(),
            );
        }
        if created_is_present && exact_matches.len() == 1 {
            if event_is_conflict_free(&post_create.events, exact_matches[0]) {
                return Ok(calendar_event_receipt(
                    created,
                    "create_conflict_free_calendar_event",
                    false,
                ));
            }
        }
        remove_calendar_event(created.event_id.clone())
            .await
            .map_err(calendar_failure_error)?;
        let remaining_events = post_create
            .events
            .into_iter()
            .filter(|event| event.event_id != created.event_id)
            .collect::<Vec<_>>();
        let concurrent_matches = remaining_events
            .iter()
            .filter(|event| existing_conflict_free_event_matches(event, &request, requested_window))
            .collect::<Vec<_>>();
        if let Some(event) = concurrent_matches.first() {
            if !event_is_conflict_free(&remaining_events, event) {
                return Err(format!(
                    "The matching '{}' event created concurrently conflicts with another Calendar event. The event from this run was removed. (calendar_existing_event_conflicted)",
                    request.title
                ));
            }
            return Ok(calendar_event_receipt(
                CalendarCreateSuccess {
                    event_id: event.event_id.clone(),
                    calendar_name: event.calendar.clone(),
                    title: event.name.clone(),
                    start_date: event.start_time.clone(),
                    end_date: event.end_time.clone(),
                    location: event.location.clone(),
                    notes: event.notes.clone(),
                    availability: event.availability,
                },
                "create_conflict_free_calendar_event",
                true,
            ));
        }
        occupied = calendar_event_intervals(&remaining_events)?;
    }
    Err(
        "Calendar kept changing while OOMU verified the selected time. No unverified event from this run remains. Try again. (calendar_conflict_window_changed)"
            .to_string(),
    )
}

pub(crate) async fn verify_conflict_free_postcondition(
    arguments: &Value,
    receipt_message: &str,
) -> Result<Value, String> {
    let validated = validate_conflict_free_registration(arguments.clone())?;
    let request =
        serde_json::from_value::<CreateConflictFreeCalendarEventRequest>(validated.arguments)
            .map_err(|_| {
                "create_conflict_free_calendar_event arguments do not match the registered schema."
                    .to_string()
            })?;
    let (receipt, requested_window) = validated_conflict_free_receipt(&request, receipt_message)?;
    let calendar = read_calendar_requiring_unique_target(
        CalendarReadRequest {
            calendar_name: String::new(),
            start_timestamp: requested_window.start_timestamp_millis as f64 / 1_000.0,
            end_timestamp: requested_window.end_timestamp_millis as f64 / 1_000.0,
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
    let exact_matches = calendar
        .events
        .iter()
        .filter(|event| existing_conflict_free_event_matches(event, &request, requested_window))
        .collect::<Vec<_>>();
    if exact_matches.len() != 1 {
        return Err(format!(
            "Calendar no longer contains exactly one matching '{}' event. (calendar_postcondition_not_unique)",
            request.title
        ));
    }
    let event = exact_matches[0];
    let receipt_matches = event.event_id == receipt.event_id
        && event.calendar == receipt.calendar_name
        && event.name == receipt.title
        && event.start_time == receipt.start_date
        && event.end_time == receipt.end_date
        && event.location == receipt.location
        && event.availability == receipt.availability
        && crate::foundation::digest::sha256_hex(event.notes.as_bytes()) == receipt.notes_sha256;
    if !receipt_matches {
        return Err(
            "Calendar's final event no longer matches the verified creation receipt. (calendar_postcondition_receipt_mismatch)"
                .to_string(),
        );
    }
    if !event_is_conflict_free(&calendar.events, event) {
        return Err(format!(
            "The final '{}' event now conflicts with another Calendar event. (calendar_postcondition_conflicted)",
            request.title
        ));
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
        "notesSha256": receipt.notes_sha256
    }))
}

fn validated_conflict_free_receipt(
    request: &CreateConflictFreeCalendarEventRequest,
    receipt_message: &str,
) -> Result<(ConflictFreeCalendarReceipt, OccupiedInterval), String> {
    if receipt_message.len() > MAX_CALENDAR_RECEIPT_BYTES {
        return Err("Calendar receipt is too large to verify safely.".to_string());
    }
    let receipt = serde_json::from_str::<ConflictFreeCalendarReceipt>(receipt_message)
        .map_err(|_| "Calendar receipt is invalid.".to_string())?;
    let start = DateTime::parse_from_rfc3339(&receipt.start_date)
        .map_err(|_| "Calendar receipt startDate is invalid.".to_string())?;
    let end = DateTime::parse_from_rfc3339(&receipt.end_date)
        .map_err(|_| "Calendar receipt endDate is invalid.".to_string())?;
    let offset = *start.offset();
    let date = start.date_naive();
    let requested_start =
        parse_conflict_free_local_time("windowStartLocal", &request.window_start_local)?;
    let requested_end =
        parse_conflict_free_local_time("windowEndLocal", &request.window_end_local)?;
    let window_start = offset
        .from_local_datetime(&date.and_time(requested_start))
        .single()
        .ok_or_else(|| "Calendar receipt window is ambiguous.".to_string())?;
    let window_end = offset
        .from_local_datetime(&date.and_time(requested_end))
        .single()
        .ok_or_else(|| "Calendar receipt window is ambiguous.".to_string())?;
    let requested_window = OccupiedInterval {
        start_timestamp_millis: window_start.timestamp_millis(),
        end_timestamp_millis: window_end.timestamp_millis(),
    };
    let receipt_is_bound = receipt.status == "completed"
        && receipt.verified
        && receipt.exists
        && receipt.created != receipt.reused_existing
        && !receipt.event_id.trim().is_empty()
        && receipt.calendar_name == request.calendar_name
        && receipt.title == request.title
        && start.timestamp_millis() >= requested_window.start_timestamp_millis
        && end.timestamp_millis() <= requested_window.end_timestamp_millis
        && end.signed_duration_since(start) == Duration::minutes(request.duration_minutes)
        && receipt.location == request.location
        && receipt.notes_verified
        && receipt.notes_sha256 == crate::foundation::digest::sha256_hex(request.notes.as_bytes())
        && receipt.availability == request.availability;
    if !receipt_is_bound {
        return Err(
            "Calendar receipt does not match the approved conflict-free event request.".to_string(),
        );
    }
    Ok((receipt, requested_window))
}

fn event_overlaps_slot(event: &CalendarEvent, slot: OccupiedInterval) -> bool {
    let Ok(start) = DateTime::parse_from_rfc3339(&event.start_time) else {
        return true;
    };
    let Ok(end) = DateTime::parse_from_rfc3339(&event.end_time) else {
        return true;
    };
    start.timestamp_millis() < slot.end_timestamp_millis
        && end.timestamp_millis() > slot.start_timestamp_millis
}

fn event_is_conflict_free(events: &[CalendarEvent], candidate: &CalendarEvent) -> bool {
    let Ok(start) = DateTime::parse_from_rfc3339(&candidate.start_time) else {
        return false;
    };
    let Ok(end) = DateTime::parse_from_rfc3339(&candidate.end_time) else {
        return false;
    };
    let slot = OccupiedInterval {
        start_timestamp_millis: start.timestamp_millis(),
        end_timestamp_millis: end.timestamp_millis(),
    };
    events.iter().all(|event| {
        event.event_id == candidate.event_id
            || !event_blocks_slot(event)
            || !event_overlaps_slot(event, slot)
    })
}

fn existing_conflict_free_event_matches(
    event: &CalendarEvent,
    request: &CreateConflictFreeCalendarEventRequest,
    requested_window: OccupiedInterval,
) -> bool {
    let Ok(start) = DateTime::parse_from_rfc3339(&event.start_time) else {
        return false;
    };
    let Ok(end) = DateTime::parse_from_rfc3339(&event.end_time) else {
        return false;
    };
    !event.event_id.is_empty()
        && event.calendar == request.calendar_name
        && event.name == request.title
        && !event.is_all_day
        && start.timestamp_millis() >= requested_window.start_timestamp_millis
        && end.timestamp_millis() <= requested_window.end_timestamp_millis
        && end.signed_duration_since(start) == Duration::minutes(request.duration_minutes)
        && event.location == request.location
        && event.notes == request.notes
        && event.availability == CalendarEventAvailability::Tentative
}

async fn create_verified_calendar_event(
    request: CalendarCreateRequest,
    operation: &'static str,
) -> Result<ExecuteCommandResponse, String> {
    let created = create_calendar_event(request)
        .await
        .map_err(calendar_failure_error)?;
    Ok(calendar_event_receipt(created, operation, false))
}

fn calendar_event_receipt(
    event: CalendarCreateSuccess,
    operation: &'static str,
    reused_existing: bool,
) -> ExecuteCommandResponse {
    let created = !reused_existing;
    let receipt = json!({
        "status": "completed",
        "verified": true,
        "exists": true,
        "created": created,
        "eventId": event.event_id,
        "calendarName": event.calendar_name,
        "title": event.title,
        "startDate": event.start_date,
        "endDate": event.end_date,
        "location": event.location,
        "notesSha256": crate::foundation::digest::sha256_hex(event.notes.as_bytes()),
        "notesVerified": true,
        "availability": event.availability,
        "reusedExisting": reused_existing,
    });
    ExecuteCommandResponse {
        operation: operation.to_string(),
        status: CommandStatus::Completed,
        message: receipt.to_string(),
        metrics: None,
        claims: vec![
            format!("CLAIM calendar_event_created={created} reused_existing={reused_existing}"),
            format!(
                "CLAIM calendar_event_verified=true exists=true event_id_sha256={}",
                crate::foundation::digest::sha256_hex(event.event_id.as_bytes())
            ),
        ],
        verified: true,
        model_used: None,
    }
}

fn execute_registration<'a>(
    context: TaskToolExecutionContext<'a>,
    arguments: Value,
) -> TaskToolFuture<'a> {
    Box::pin(async move {
        let request = serde_json::from_value::<CreateSystemCalendarEventRequest>(arguments)
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
            "create_system_calendar_event",
        )
        .await
    })
}

fn execute_conflict_free_registration<'a>(
    context: TaskToolExecutionContext<'a>,
    arguments: Value,
) -> TaskToolFuture<'a> {
    Box::pin(async move {
        let request = serde_json::from_value::<CreateConflictFreeCalendarEventRequest>(arguments)
            .map_err(|_| {
            "create_conflict_free_calendar_event arguments do not match the registered schema."
                .to_string()
        })?;
        let execution_id = context.execution_id.ok_or_else(|| {
            "Creating a conflict-free Calendar event requires an active approved Task.".to_string()
        })?;
        require_agent_runtime_task(context.persistence, execution_id)?;
        create_conflict_free_calendar_event(request).await
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::task_tool_runtime;
    use chrono::Timelike;

    fn request() -> Value {
        json!({
            "calendarName": "OOMU Test",
            "title": "Supplier Decision Review",
            "startDate": "2026-07-20T14:00:00-04:00",
            "endDate": "2026-07-20T15:00:00-04:00",
            "location": "Video conference",
            "notes": "Review the verified decision pack.",
            "availability": "tentative"
        })
    }

    fn conflict_free_request() -> Value {
        json!({
            "calendarName": "OOMU Test",
            "title": "Supplier Decision Review",
            "day": "next_weekday",
            "windowStartLocal": "13:00",
            "windowEndLocal": "16:00",
            "durationMinutes": 30,
            "location": "",
            "notes": "Review the verified decision pack.",
            "availability": "tentative"
        })
    }

    #[test]
    fn schema_has_only_the_exact_bounded_event_fields() {
        let schema = create_system_calendar_event_schema();
        let properties = schema["properties"].as_object().unwrap();
        assert_eq!(properties.len(), 7);
        for field in [
            "calendarName",
            "title",
            "startDate",
            "endDate",
            "location",
            "notes",
            "availability",
        ] {
            assert!(properties.contains_key(field));
            assert!(schema["required"]
                .as_array()
                .unwrap()
                .contains(&Value::String(field.to_string())));
        }
        assert_eq!(schema["additionalProperties"], false);
    }

    #[test]
    fn conflict_free_schema_exposes_a_bounded_user_selected_window() {
        let schema = create_conflict_free_calendar_event_schema();
        let properties = schema["properties"].as_object().unwrap();
        assert_eq!(properties.len(), 9);
        for field in [
            "calendarName",
            "title",
            "day",
            "windowStartLocal",
            "windowEndLocal",
            "durationMinutes",
            "location",
            "notes",
            "availability",
        ] {
            assert!(properties.contains_key(field));
            assert!(schema["required"]
                .as_array()
                .unwrap()
                .contains(&Value::String(field.to_string())));
        }
        assert_eq!(properties["day"]["enum"], json!(["next_weekday"]));
        assert_eq!(
            properties["windowStartLocal"]["pattern"],
            "^(?:0[6-9]|1[0-9]|2[0-1]):[0-5][0-9]$"
        );
        assert_eq!(
            properties["windowEndLocal"]["pattern"],
            "^(?:0[6-9]|1[0-9]|2[0-2]):[0-5][0-9]$"
        );
        assert_eq!(properties["durationMinutes"]["enum"], json!([30]));
        assert_eq!(properties["availability"]["enum"], json!(["tentative"]));
        assert_eq!(schema["additionalProperties"], false);
    }

    #[test]
    fn validation_normalizes_dates_and_is_always_effectful() {
        let validated = validate_registration(request()).unwrap();
        assert!(validated.potentially_effectful);
        assert_eq!(
            validated.arguments["startDate"],
            "2026-07-20T14:00:00-04:00"
        );
        assert_eq!(validated.arguments["availability"], "tentative");
    }

    #[test]
    fn validation_rejects_unbounded_or_ambiguous_event_requests() {
        let mut reversed = request();
        reversed["endDate"] = json!("2026-07-20T13:00:00-04:00");
        assert!(validate_registration(reversed).is_err());

        let mut too_long = request();
        too_long["endDate"] = json!("2026-07-21T14:00:01-04:00");
        assert!(validate_registration(too_long).is_err());

        let mut too_precise = request();
        too_precise["startDate"] = json!("2026-07-20T14:00:00.000001-04:00");
        assert!(validate_registration(too_precise).is_err());

        let mut unknown = request();
        unknown["invitees"] = json!(["person@example.com"]);
        assert!(validate_registration(unknown).is_err());

        let mut unsupported = request();
        unsupported["availability"] = json!("unavailable");
        assert!(validate_registration(unsupported).is_err());
    }

    fn interval(start: i64, end: i64) -> OccupiedInterval {
        OccupiedInterval {
            start_timestamp_millis: start,
            end_timestamp_millis: end,
        }
    }

    fn conflict_free_request_value() -> CreateConflictFreeCalendarEventRequest {
        serde_json::from_value(conflict_free_request()).unwrap()
    }

    fn calendar_event(
        event_id: &str,
        start_time: &str,
        end_time: &str,
        availability: CalendarEventAvailability,
    ) -> CalendarEvent {
        CalendarEvent {
            event_id: event_id.to_string(),
            calendar: "OOMU Test".to_string(),
            name: "Supplier Decision Review".to_string(),
            start_time: start_time.to_string(),
            end_time: end_time.to_string(),
            location: String::new(),
            notes: "Review the verified decision pack.".to_string(),
            availability,
            is_all_day: false,
            declined_by_current_user: false,
            time_zone: "America/New_York".to_string(),
        }
    }

    #[test]
    fn next_weekday_skips_weekends_deterministically() {
        let thursday = NaiveDate::from_ymd_opt(2026, 7, 16).unwrap();
        let friday = NaiveDate::from_ymd_opt(2026, 7, 17).unwrap();
        let saturday = NaiveDate::from_ymd_opt(2026, 7, 18).unwrap();
        let sunday = NaiveDate::from_ymd_opt(2026, 7, 19).unwrap();
        assert_eq!(
            next_weekday_after(thursday),
            NaiveDate::from_ymd_opt(2026, 7, 17)
        );
        for date in [friday, saturday, sunday] {
            assert_eq!(
                next_weekday_after(date),
                NaiveDate::from_ymd_opt(2026, 7, 20)
            );
        }
    }

    #[test]
    fn local_window_is_exactly_one_until_four_pm() {
        let date = NaiveDate::from_ymd_opt(2026, 7, 20).unwrap();
        let (start, end) = local_conflict_free_window(
            date,
            CONFLICT_FREE_WINDOW_START_LOCAL,
            CONFLICT_FREE_WINDOW_END_LOCAL,
        )
        .unwrap();
        assert_eq!(start.date_naive(), date);
        assert_eq!(end.date_naive(), date);
        assert_eq!(start.hour(), CONFLICT_FREE_WINDOW_START_HOUR);
        assert_eq!(end.hour(), CONFLICT_FREE_WINDOW_END_HOUR);
        assert_eq!(end.signed_duration_since(start), Duration::hours(3));
    }

    #[test]
    fn slot_selection_returns_the_window_start_when_it_is_free() {
        assert_eq!(
            earliest_non_overlapping_slot(0, 180, 30, &[]),
            Some(interval(0, 30))
        );
        assert_eq!(
            earliest_non_overlapping_slot(0, 180, 30, &[interval(-30, 0), interval(30, 60)]),
            Some(interval(0, 30)),
            "events touching either edge do not overlap the candidate"
        );
    }

    #[test]
    fn slot_selection_is_order_independent_and_uses_the_earliest_gap() {
        let occupied = [
            interval(120, 150),
            interval(80, 100),
            interval(15, 50),
            interval(0, 20),
        ];
        assert_eq!(
            earliest_non_overlapping_slot(0, 180, 30, &occupied),
            Some(interval(50, 80))
        );
    }

    #[test]
    fn slot_selection_returns_none_when_the_window_cannot_fit_the_duration() {
        assert_eq!(
            earliest_non_overlapping_slot(0, 180, 30, &[interval(-10, 160)]),
            None
        );
        assert_eq!(earliest_non_overlapping_slot(0, 29, 30, &[]), None);
        assert_eq!(earliest_non_overlapping_slot(0, 180, 0, &[]), None);
    }

    #[test]
    fn existing_event_reuse_requires_the_whole_non_all_day_event_inside_the_window() {
        let request = conflict_free_request_value();
        let window = interval(
            DateTime::parse_from_rfc3339("2026-07-20T13:00:00-04:00")
                .unwrap()
                .timestamp_millis(),
            DateTime::parse_from_rfc3339("2026-07-20T16:00:00-04:00")
                .unwrap()
                .timestamp_millis(),
        );
        let valid = calendar_event(
            "valid",
            "2026-07-20T13:00:00-04:00",
            "2026-07-20T13:30:00-04:00",
            CalendarEventAvailability::Tentative,
        );
        assert!(existing_conflict_free_event_matches(
            &valid, &request, window
        ));

        let starts_before_window = calendar_event(
            "early",
            "2026-07-20T12:45:00-04:00",
            "2026-07-20T13:15:00-04:00",
            CalendarEventAvailability::Tentative,
        );
        assert!(!existing_conflict_free_event_matches(
            &starts_before_window,
            &request,
            window
        ));

        let ends_after_window = calendar_event(
            "late",
            "2026-07-20T15:45:00-04:00",
            "2026-07-20T16:15:00-04:00",
            CalendarEventAvailability::Tentative,
        );
        assert!(!existing_conflict_free_event_matches(
            &ends_after_window,
            &request,
            window
        ));

        let mut all_day = valid;
        all_day.event_id = "all-day".to_string();
        all_day.is_all_day = true;
        assert!(!existing_conflict_free_event_matches(
            &all_day, &request, window
        ));
    }

    #[test]
    fn existing_event_reuse_rechecks_conflicts_across_all_calendars() {
        let candidate = calendar_event(
            "candidate",
            "2026-07-20T13:00:00-04:00",
            "2026-07-20T13:30:00-04:00",
            CalendarEventAvailability::Tentative,
        );
        let mut conflict = calendar_event(
            "conflict",
            "2026-07-20T13:15:00-04:00",
            "2026-07-20T13:45:00-04:00",
            CalendarEventAvailability::Busy,
        );
        conflict.calendar = "Work".to_string();
        conflict.name = "Existing meeting".to_string();
        assert!(!event_is_conflict_free(
            &[candidate.clone(), conflict.clone()],
            &candidate
        ));
        conflict.declined_by_current_user = true;
        assert!(event_is_conflict_free(
            &[candidate.clone(), conflict.clone()],
            &candidate
        ));

        conflict.declined_by_current_user = false;
        conflict.availability = CalendarEventAvailability::Free;
        assert!(!event_is_conflict_free(
            &[candidate.clone(), conflict],
            &candidate
        ));

        let mut touching = calendar_event(
            "touching",
            "2026-07-20T13:30:00-04:00",
            "2026-07-20T14:00:00-04:00",
            CalendarEventAvailability::Busy,
        );
        touching.calendar = "Work".to_string();
        touching.name = "Next meeting".to_string();
        assert!(event_is_conflict_free(
            &[candidate.clone(), touching],
            &candidate
        ));
    }

    #[test]
    fn conflict_free_request_accepts_scenario_six_at_two_pm_or_later() {
        let mut request = conflict_free_request();
        request["calendarName"] = json!("  OOMU Test ");
        request["title"] = json!(" Supplier Decision Review ");
        request["notes"] = json!(" Review the verified decision pack. ");
        let validated = validate_conflict_free_registration(request).unwrap();
        assert!(validated.potentially_effectful);
        assert_eq!(validated.arguments["calendarName"], "OOMU Test");
        assert_eq!(validated.arguments["title"], "Supplier Decision Review");
        assert_eq!(
            validated.arguments["notes"],
            "Review the verified decision pack."
        );

        let mut scenario_six = conflict_free_request();
        scenario_six["title"] = json!("Supplier Exception Follow-up");
        scenario_six["windowStartLocal"] = json!("14:00");
        scenario_six["windowEndLocal"] = json!("18:00");
        let scenario_six = validate_conflict_free_registration(scenario_six).unwrap();
        assert_eq!(scenario_six.arguments["windowStartLocal"], "14:00");
        assert_eq!(scenario_six.arguments["windowEndLocal"], "18:00");

        for (field, invalid) in [
            ("day", json!("today")),
            ("windowStartLocal", json!("05:59")),
            ("windowEndLocal", json!("22:01")),
            ("durationMinutes", json!(45)),
            ("availability", json!("busy")),
        ] {
            let mut request = conflict_free_request();
            request[field] = invalid;
            assert!(validate_conflict_free_registration(request).is_err());
        }
        let mut unknown = conflict_free_request();
        unknown["invitees"] = json!(["person@example.com"]);
        assert!(validate_conflict_free_registration(unknown).is_err());
    }

    #[test]
    fn shared_receipt_shape_remains_verified_and_hashes_notes() {
        let response = calendar_event_receipt(
            CalendarCreateSuccess {
                event_id: "event-123".to_string(),
                calendar_name: "OOMU Test".to_string(),
                title: "Supplier Decision Review".to_string(),
                start_date: "2026-07-20T13:00:00-04:00".to_string(),
                end_date: "2026-07-20T13:30:00-04:00".to_string(),
                location: String::new(),
                notes: "Review the verified decision pack.".to_string(),
                availability: CalendarEventAvailability::Tentative,
            },
            "create_system_calendar_event",
            false,
        );
        let receipt: Value = serde_json::from_str(&response.message).unwrap();
        assert_eq!(response.operation, "create_system_calendar_event");
        assert!(response.verified);
        assert_eq!(receipt["status"], "completed");
        assert_eq!(receipt["verified"], true);
        assert_eq!(receipt["exists"], true);
        assert_eq!(receipt["created"], true);
        assert_eq!(receipt["availability"], "tentative");
        assert_eq!(receipt["notesVerified"], true);
        assert_eq!(receipt["reusedExisting"], false);
        assert!(response.claims[0].contains("calendar_event_created=true"));
        assert!(response.claims[0].contains("reused_existing=false"));
        assert!(response.claims[1].contains("calendar_event_verified=true exists=true"));
        assert_eq!(
            receipt["notesSha256"],
            crate::foundation::digest::sha256_hex("Review the verified decision pack.".as_bytes())
        );
    }

    #[test]
    fn reused_existing_receipt_never_claims_that_the_event_was_created() {
        let response = calendar_event_receipt(
            CalendarCreateSuccess {
                event_id: "event-existing-123".to_string(),
                calendar_name: "OOMU Test".to_string(),
                title: "Supplier Decision Review".to_string(),
                start_date: "2026-07-20T13:00:00-04:00".to_string(),
                end_date: "2026-07-20T13:30:00-04:00".to_string(),
                location: String::new(),
                notes: "Review the verified decision pack.".to_string(),
                availability: CalendarEventAvailability::Tentative,
            },
            "create_conflict_free_calendar_event",
            true,
        );
        let receipt: Value = serde_json::from_str(&response.message).unwrap();

        assert!(matches!(response.status, CommandStatus::Completed));
        assert!(response.verified);
        assert_eq!(receipt["exists"], true);
        assert_eq!(receipt["verified"], true);
        assert_eq!(receipt["created"], false);
        assert_eq!(receipt["reusedExisting"], true);
        assert!(response.claims[0].contains("calendar_event_created=false"));
        assert!(response.claims[0].contains("reused_existing=true"));
        assert!(!response
            .claims
            .iter()
            .any(|claim| claim.contains("calendar_event_created=true")));
        assert!(response.claims[1].contains("calendar_event_verified=true exists=true"));
    }

    #[test]
    fn final_receipt_validation_is_bound_to_the_approved_window_and_notes() {
        let request = conflict_free_request_value();
        let response = calendar_event_receipt(
            CalendarCreateSuccess {
                event_id: "event-final-123".to_string(),
                calendar_name: request.calendar_name.clone(),
                title: request.title.clone(),
                start_date: "2026-07-20T13:00:00-04:00".to_string(),
                end_date: "2026-07-20T13:30:00-04:00".to_string(),
                location: request.location.clone(),
                notes: request.notes.clone(),
                availability: request.availability,
            },
            "create_conflict_free_calendar_event",
            false,
        );
        let (_, window) = validated_conflict_free_receipt(&request, &response.message).unwrap();
        assert_eq!(
            window.start_timestamp_millis,
            DateTime::parse_from_rfc3339("2026-07-20T13:00:00-04:00")
                .unwrap()
                .timestamp_millis()
        );

        let mut mismatched: Value = serde_json::from_str(&response.message).unwrap();
        mismatched["notesSha256"] = json!("0".repeat(64));
        assert!(validated_conflict_free_receipt(&request, &mismatched.to_string()).is_err());

        let mut outside_window: Value = serde_json::from_str(&response.message).unwrap();
        outside_window["startDate"] = json!("2026-07-20T12:45:00-04:00");
        outside_window["endDate"] = json!("2026-07-20T13:15:00-04:00");
        assert!(validated_conflict_free_receipt(&request, &outside_window.to_string()).is_err());
    }

    #[test]
    fn registration_is_visible_and_requires_explicit_approval() {
        let _ = register_task_tool();
        let validated = task_tool_runtime::validate("create_system_calendar_event", request())
            .expect("registered Calendar mutation");
        assert!(validated.potentially_effectful());
        assert_eq!(
            task_tool_runtime::approval_tier("create_system_calendar_event"),
            Some(TaskToolApprovalTier::Explicit)
        );
        assert!(task_tool_runtime::schema("create_system_calendar_event").is_ok());
        let registry = crate::tools::registry::NativeToolRegistry::default();
        assert!(
            registry.schema_payload(crate::tools::registry::ModelProvider::LocalGemmaIt)["tools"]
                .as_array()
                .is_some_and(|tools| tools
                    .iter()
                    .any(|tool| tool["kind"] == "create_system_calendar_event"))
        );
        let planned = task_tool_runtime::PlannedTaskToolRequest::new(
            "create_system_calendar_event",
            validated.arguments,
        );
        let action = task_tool_runtime::requested_action(&planned);
        let approval = crate::shield_gate::build_shield_approval_request(&action)
            .expect("Calendar mutation has explicit Shield semantics");
        assert_eq!(approval.approval_tier, "explicit_confirmation");
        assert!(crate::shield_gate::authorize_action(action.clone()).is_err());
        assert!(crate::shield_gate::authorize_action_for_approved_plan(action).is_ok());

        let conflict_free = task_tool_runtime::validate(
            "create_conflict_free_calendar_event",
            conflict_free_request(),
        )
        .expect("registered conflict-free Calendar mutation");
        assert!(conflict_free.potentially_effectful());
        assert_eq!(
            task_tool_runtime::approval_tier("create_conflict_free_calendar_event"),
            Some(TaskToolApprovalTier::Explicit)
        );
        assert!(task_tool_runtime::schema("create_conflict_free_calendar_event").is_ok());
        assert!(
            registry.schema_payload(crate::tools::registry::ModelProvider::LocalGemmaIt)["tools"]
                .as_array()
                .is_some_and(|tools| tools
                    .iter()
                    .any(|tool| tool["kind"] == "create_conflict_free_calendar_event"))
        );
        let planned = task_tool_runtime::PlannedTaskToolRequest::new(
            "create_conflict_free_calendar_event",
            conflict_free.arguments,
        );
        let action = task_tool_runtime::requested_action(&planned);
        let approval = crate::shield_gate::build_shield_approval_request(&action)
            .expect("conflict-free Calendar mutation has explicit Shield semantics");
        assert_eq!(approval.approval_tier, "explicit_confirmation");
        assert!(approval.mandatory_reconfirm);
        assert_eq!(approval.approval_scope_kinds, vec!["once"]);
    }
}
