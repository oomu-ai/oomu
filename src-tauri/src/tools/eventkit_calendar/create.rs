use super::{
    broker::CalendarOperationTrace, failure, require_active_calendar_read,
    require_current_full_calendar_access, target_failure, CalendarCreateRequest,
    CalendarCreateSuccess, CalendarEventAvailability, CalendarReadFailure,
};
use objc2::rc::{autoreleasepool, Retained};
use objc2::AnyThread;
use objc2_event_kit::{
    EKCalendar, EKCalendarEventAvailabilityMask, EKEntityType, EKErrorCode, EKEvent,
    EKEventAvailability, EKEventStore, EKSource, EKSourceType, EKSpan,
};
use objc2_foundation::{NSDate, NSString};
use std::collections::HashSet;

#[derive(Clone, Debug, Eq, PartialEq)]
struct CalendarSourceDescriptor {
    identifier: String,
    title: String,
    source_type: isize,
    is_delegate: bool,
    is_default: bool,
    supports_required_availability: bool,
}

fn calendar_source_is_eligible(source: &CalendarSourceDescriptor) -> bool {
    !source.is_delegate
        && source.source_type != EKSourceType::Subscribed.0
        && source.source_type != EKSourceType::Birthdays.0
}

fn calendar_source_rank(source: &CalendarSourceDescriptor) -> (u8, u8, u8, String, String) {
    let type_rank = if source.source_type == EKSourceType::Local.0 {
        0
    } else if source.source_type == EKSourceType::CalDAV.0
        || source.source_type == EKSourceType::MobileMe.0
    {
        1
    } else if source.source_type == EKSourceType::Exchange.0 {
        2
    } else {
        3
    };
    (
        u8::from(!source.supports_required_availability),
        u8::from(!source.is_default),
        type_rank,
        source.title.to_lowercase(),
        source.identifier.clone(),
    )
}

fn ranked_calendar_sources(
    store: &EKEventStore,
    required_availability: CalendarEventAvailability,
) -> Vec<Retained<EKSource>> {
    let default_source_identifier = unsafe {
        store
            .defaultCalendarForNewEvents()
            .and_then(|calendar| calendar.source())
            .map(|source| source.sourceIdentifier().to_string())
    };
    let required_mask = availability_mask(required_availability);
    let compatible_source_identifiers = unsafe {
        store
            .calendarsForEntityType(EKEntityType::Event)
            .to_vec()
            .into_iter()
            .filter(|calendar| {
                calendar.allowsContentModifications()
                    && calendar
                        .supportedEventAvailabilities()
                        .contains(required_mask)
            })
            .filter_map(|calendar| calendar.source())
            .map(|source| source.sourceIdentifier().to_string())
            .collect::<HashSet<_>>()
    };
    let mut candidates = unsafe {
        store
            .sources()
            .to_vec()
            .into_iter()
            .map(|source| {
                let identifier = source.sourceIdentifier().to_string();
                let supports_required_availability =
                    compatible_source_identifiers.contains(&identifier);
                let descriptor = CalendarSourceDescriptor {
                    is_default: default_source_identifier.as_deref() == Some(identifier.as_str()),
                    identifier,
                    title: source.title().to_string(),
                    source_type: source.sourceType().0,
                    is_delegate: source.isDelegate(),
                    supports_required_availability,
                };
                (source, descriptor)
            })
            .filter(|(_, descriptor)| calendar_source_is_eligible(descriptor))
            .collect::<Vec<_>>()
    };
    candidates.sort_by_key(|(_, descriptor)| calendar_source_rank(descriptor));
    let mut seen = HashSet::new();
    candidates.retain(|(_, descriptor)| {
        descriptor.identifier.is_empty() || seen.insert(descriptor.identifier.clone())
    });
    candidates.into_iter().map(|(source, _)| source).collect()
}

pub(super) fn calendar_supports_availability(
    calendar: &EKCalendar,
    availability: CalendarEventAvailability,
) -> bool {
    unsafe {
        calendar
            .supportedEventAvailabilities()
            .contains(availability_mask(availability))
    }
}

fn compatible_calendar_names(
    store: &EKEventStore,
    availability: CalendarEventAvailability,
) -> Vec<String> {
    unsafe {
        super::bounded_compatible_calendar_names(
            store
                .calendarsForEntityType(EKEntityType::Event)
                .to_vec()
                .iter()
                .map(|calendar| {
                    (
                        calendar.title().to_string(),
                        calendar.allowsContentModifications(),
                        calendar_supports_availability(calendar, availability),
                    )
                }),
        )
    }
}

pub(super) fn compatible_calendar_names_blocking(
    required_availability: CalendarEventAvailability,
    trace: &CalendarOperationTrace,
) -> Result<Vec<String>, CalendarReadFailure> {
    autoreleasepool(|_| unsafe {
        require_active_calendar_read(trace)?;
        let store = EKEventStore::init(EKEventStore::alloc());
        require_current_full_calendar_access()?;
        require_active_calendar_read(trace)?;
        let names = compatible_calendar_names(&store, required_availability);
        require_active_calendar_read(trace)?;
        Ok(names)
    })
}

fn source_capability_error(domain: &str, code: isize) -> bool {
    domain == "EKErrorDomain"
        && matches!(
            code,
            value if value == EKErrorCode::CalendarSourceCannotBeModified.0
                || value == EKErrorCode::SourceDoesNotAllowCalendarAddDelete.0
                || value == EKErrorCode::CalendarDoesNotAllowEvents.0
                || value == EKErrorCode::SourceDoesNotAllowEvents.0
        )
}

pub(super) fn create_calendar_event_blocking(
    request: CalendarCreateRequest,
    trace: &CalendarOperationTrace,
) -> Result<CalendarCreateSuccess, CalendarReadFailure> {
    autoreleasepool(|_| unsafe {
        let store = EKEventStore::init(EKEventStore::alloc());
        require_current_full_calendar_access()?;

        let mut matching_calendars = store
            .calendarsForEntityType(EKEntityType::Event)
            .to_vec()
            .into_iter()
            .filter(|calendar| calendar.title().to_string() == request.calendar_name)
            .collect::<Vec<_>>();
        if matching_calendars.is_empty() {
            return Err(target_failure(
                "calendar_not_found",
                "The exact requested calendar was not found.",
                &request.calendar_name,
                compatible_calendar_names(&store, request.availability),
            ));
        }
        if matching_calendars.len() != 1 {
            return Err(target_failure(
                "calendar_name_ambiguous",
                "More than one calendar has the exact requested name.",
                &request.calendar_name,
                compatible_calendar_names(&store, request.availability),
            ));
        }
        let calendar = matching_calendars.pop().expect("one matching calendar");
        if !calendar.allowsContentModifications() {
            return Err(target_failure(
                "calendar_read_only",
                "The exact requested calendar is read-only.",
                &request.calendar_name,
                compatible_calendar_names(&store, request.availability),
            ));
        }
        if !calendar
            .supportedEventAvailabilities()
            .contains(availability_mask(request.availability))
        {
            return Err(target_failure(
                "calendar_availability_unsupported",
                "The exact requested calendar cannot represent the event availability required by this task.",
                &request.calendar_name,
                compatible_calendar_names(&store, request.availability),
            ));
        }

        let event = EKEvent::eventWithEventStore(&store);
        let title = NSString::from_str(&request.title);
        let location = NSString::from_str(&request.location);
        let notes = NSString::from_str(&request.notes);
        let start =
            NSDate::dateWithTimeIntervalSince1970(request.start_timestamp_millis as f64 / 1_000.0);
        let end =
            NSDate::dateWithTimeIntervalSince1970(request.end_timestamp_millis as f64 / 1_000.0);
        event.setCalendar(Some(&calendar));
        event.setTitle(Some(&title));
        event.setStartDate(Some(&start));
        event.setEndDate(Some(&end));
        event.setAllDay(false);
        event.setLocation(Some(&location));
        event.setNotes(Some(&notes));
        event.setAvailability(native_availability(request.availability));
        trace.require_not_cancelled()?;
        store
            .saveEvent_span_error(&event, EKSpan::ThisEvent)
            .map_err(|_| {
                failure(
                    "calendar_event_save_failed",
                    "The approved Calendar event could not be saved.",
                    true,
                )
            })?;

        let saved_identifier_for_cleanup = event.eventIdentifier();
        let verification = (|| {
            let event_id = event
                .eventIdentifier()
                .map(|identifier| identifier.to_string())
                .filter(|identifier| !identifier.is_empty())
                .ok_or_else(|| {
                    failure(
                        "calendar_event_identifier_missing",
                        "Calendar did not return an identifier for the saved event.",
                        true,
                    )
                })?;
            let identifier = NSString::from_str(&event_id);
            let saved = store.eventWithIdentifier(&identifier).ok_or_else(|| {
                failure(
                    "calendar_event_readback_failed",
                    "The saved Calendar event could not be read back.",
                    true,
                )
            })?;
            let saved_calendar = saved
                .calendar()
                .map(|value| value.title().to_string())
                .unwrap_or_default();
            let saved_title = saved.title().to_string();
            let saved_location = saved
                .location()
                .map(|value| value.to_string())
                .unwrap_or_default();
            let saved_notes = saved
                .notes()
                .map(|value| value.to_string())
                .unwrap_or_default();
            let verified = saved_calendar == request.calendar_name
                && saved_title == request.title
                && timestamp_millis(&saved.startDate())? == request.start_timestamp_millis
                && timestamp_millis(&saved.endDate())? == request.end_timestamp_millis
                && saved.availability() == native_availability(request.availability)
                && saved_location == request.location
                && saved_notes == request.notes;
            if !verified {
                return Err(failure(
                    "calendar_event_verification_failed",
                    "The saved Calendar event did not match the approved request.",
                    true,
                ));
            }
            Ok(CalendarCreateSuccess {
                event_id,
                calendar_name: request.calendar_name.clone(),
                title: request.title.clone(),
                start_date: request.start_date.clone(),
                end_date: request.end_date.clone(),
                location: request.location.clone(),
                notes: request.notes.clone(),
                availability: request.availability,
            })
        })();
        stop_cancelled_event_mutation(
            trace,
            &store,
            &event,
            saved_identifier_for_cleanup.as_deref(),
        )?;
        if verification.is_err() {
            let removed = store
                .removeEvent_span_error(&event, EKSpan::ThisEvent)
                .is_ok();
            let absent_after_removal = saved_identifier_for_cleanup
                .as_deref()
                .is_none_or(|identifier| store.eventWithIdentifier(identifier).is_none());
            if !removed || !absent_after_removal {
                return Err(failure(
                    "calendar_event_cleanup_failed",
                    "Calendar could not verify the saved event or remove the unverified event. Review Calendar before retrying.",
                    false,
                ));
            }
        }
        verification
    })
}

pub(super) fn validate_calendar_target_blocking(
    requested_name: &str,
    required_availability: CalendarEventAvailability,
    trace: &CalendarOperationTrace,
) -> Result<(), CalendarReadFailure> {
    autoreleasepool(|_| unsafe {
        require_active_calendar_read(trace)?;
        let store = EKEventStore::init(EKEventStore::alloc());
        require_current_full_calendar_access()?;
        require_active_calendar_read(trace)?;
        let matching = store
            .calendarsForEntityType(EKEntityType::Event)
            .to_vec()
            .into_iter()
            .filter(|calendar| calendar.title().to_string() == requested_name)
            .collect::<Vec<_>>();
        require_active_calendar_read(trace)?;
        let alternatives = || compatible_calendar_names(&store, required_availability);
        let calendar = match matching.as_slice() {
            [] => {
                return Err(target_failure(
                    "calendar_not_found",
                    "The selected calendar is no longer available.",
                    requested_name,
                    alternatives(),
                ))
            }
            [calendar] => calendar,
            _ => {
                return Err(target_failure(
                    "calendar_name_ambiguous",
                    "More than one calendar now has the selected name.",
                    requested_name,
                    alternatives(),
                ))
            }
        };
        if !calendar.allowsContentModifications() {
            return Err(target_failure(
                "calendar_read_only",
                "The selected calendar is no longer writable.",
                requested_name,
                alternatives(),
            ));
        }
        if !calendar_supports_availability(calendar, required_availability) {
            return Err(target_failure(
                "calendar_availability_unsupported",
                "The selected calendar cannot represent the event availability required by this task.",
                requested_name,
                alternatives(),
            ));
        }
        Ok(())
    })
}

pub(super) fn create_calendar_blocking(
    requested_name: &str,
    required_availability: CalendarEventAvailability,
    trace: &CalendarOperationTrace,
) -> Result<Option<String>, CalendarReadFailure> {
    autoreleasepool(|_| unsafe {
        let requested_name = require_calendar_name(requested_name)?;
        let store = EKEventStore::init(EKEventStore::alloc());
        let Some(sources) =
            prepare_calendar_creation(&store, requested_name, required_availability)?
        else {
            return Ok(None);
        };

        let title = NSString::from_str(requested_name);
        let mut rejected_source_count = 0usize;
        for source in sources {
            let calendar =
                EKCalendar::calendarForEntityType_eventStore(EKEntityType::Event, &store);
            calendar.setTitle(&title);
            calendar.setSource(Some(&source));
            trace.require_not_cancelled()?;
            if let Err(error) = store.saveCalendar_commit_error(&calendar, true) {
                let domain = error.domain().to_string();
                let code = error.code();
                let retryable = source_capability_error(&domain, code);
                eprintln!(
                    "OOMU_CALENDAR_CREATE_FAILURE domain={} code={} source_type={} retryable={}",
                    domain,
                    code,
                    source.sourceType().0,
                    retryable
                );

                // EventKit can report a commit error after the account accepted
                // the object. A verified readback is authoritative and prevents
                // a retry from creating a duplicate calendar.
                let mut readback = store
                    .calendarsForEntityType(EKEntityType::Event)
                    .to_vec()
                    .into_iter()
                    .filter(|candidate| candidate.title().to_string() == requested_name)
                    .collect::<Vec<_>>();
                stop_cancelled_calendar_mutation(trace, &store, requested_name, &readback)?;
                if readback.len() == 1 && readback[0].allowsContentModifications() {
                    if calendar_supports_availability(&readback[0], required_availability) {
                        let calendar_id = readback[0].calendarIdentifier().to_string();
                        if !calendar_id.is_empty() {
                            return Ok(Some(calendar_id));
                        }
                    } else if retryable {
                        let incompatible = readback.pop().expect("one incompatible calendar");
                        if !remove_calendar_and_verify(&store, &incompatible) {
                            return Err(failure(
                                "calendar_cleanup_failed",
                                "Calendar could not remove an incompatible calendar created during recovery. Review Calendar before continuing.",
                                false,
                            ));
                        }
                        rejected_source_count += 1;
                        continue;
                    }
                }
                if readback.len() > 1 {
                    return Err(target_failure(
                        "calendar_name_ambiguous",
                        "More than one calendar now has the requested name. Choose the intended calendar before continuing.",
                        requested_name,
                        compatible_calendar_names(&store, required_availability),
                    ));
                }
                if retryable {
                    rejected_source_count += 1;
                    continue;
                }
                return Err(failure(
                    "calendar_create_failed",
                    "Calendar could not create the approved calendar. Choose an existing calendar or try again.",
                    false,
                ));
            }

            let calendar_id = calendar.calendarIdentifier().to_string();
            let saved = (!calendar_id.is_empty())
                .then(|| {
                    let identifier = NSString::from_str(&calendar_id);
                    store.calendarWithIdentifier(&identifier)
                })
                .flatten();
            let saved_calendars = store
                .calendarsForEntityType(EKEntityType::Event)
                .to_vec()
                .into_iter()
                .filter(|candidate| candidate.title().to_string() == requested_name)
                .collect::<Vec<_>>();
            stop_cancelled_calendar_mutation(trace, &store, requested_name, &saved_calendars)?;
            let exact_count = store
                .calendarsForEntityType(EKEntityType::Event)
                .to_vec()
                .iter()
                .filter(|candidate| candidate.title().to_string() == requested_name)
                .count();
            let saved_supports_required_availability = saved.as_ref().is_some_and(|candidate| {
                calendar_supports_availability(candidate, required_availability)
            });
            let verified = saved.as_ref().is_some_and(|candidate| {
                candidate.title().to_string() == requested_name
                    && candidate.allowsContentModifications()
                    && saved_supports_required_availability
                    && candidate.source().is_some_and(|saved_source| {
                        saved_source.sourceIdentifier().to_string()
                            == source.sourceIdentifier().to_string()
                    })
            }) && exact_count == 1;
            if verified {
                let final_readback = store
                    .calendarsForEntityType(EKEntityType::Event)
                    .to_vec()
                    .into_iter()
                    .filter(|candidate| candidate.title().to_string() == requested_name)
                    .collect::<Vec<_>>();
                stop_cancelled_calendar_mutation(trace, &store, requested_name, &final_readback)?;
                return Ok(Some(calendar_id));
            }
            if !remove_calendar_and_verify(&store, &calendar) {
                return Err(failure(
                    "calendar_cleanup_failed",
                    "Calendar could not verify or remove the new calendar. Review Calendar before continuing.",
                    false,
                ));
            }
            if !saved_supports_required_availability {
                rejected_source_count += 1;
                continue;
            }
            return Err(failure(
                "calendar_create_verification_failed",
                "The new calendar could not be verified and was removed.",
                false,
            ));
        }

        Err(failure(
            "calendar_source_unavailable",
            &format!(
                "Calendar could not create the approved calendar in any available account ({} checked). Choose an existing calendar instead.",
                rejected_source_count
            ),
            false,
        ))
    })
}

fn require_calendar_name(value: &str) -> Result<&str, CalendarReadFailure> {
    let value = value.trim();
    if value.is_empty() {
        Err(failure(
            "calendar_name_invalid",
            "A calendar name is required.",
            false,
        ))
    } else {
        Ok(value)
    }
}

fn require_calendar_sources(
    sources: Vec<Retained<EKSource>>,
) -> Result<Vec<Retained<EKSource>>, CalendarReadFailure> {
    if sources.is_empty() {
        Err(failure(
            "calendar_source_unavailable",
            "Calendar did not provide an account that can hold the new calendar.",
            false,
        ))
    } else {
        Ok(sources)
    }
}

fn prepare_calendar_creation(
    store: &EKEventStore,
    requested_name: &str,
    required_availability: CalendarEventAvailability,
) -> Result<Option<Vec<Retained<EKSource>>>, CalendarReadFailure> {
    require_current_full_calendar_access()?;
    if existing_calendar_is_usable(store, requested_name, required_availability)? {
        Ok(None)
    } else {
        require_calendar_sources(ranked_calendar_sources(store, required_availability)).map(Some)
    }
}

fn existing_calendar_is_usable(
    store: &EKEventStore,
    requested_name: &str,
    required_availability: CalendarEventAvailability,
) -> Result<bool, CalendarReadFailure> {
    let matching = unsafe {
        store
            .calendarsForEntityType(EKEntityType::Event)
            .to_vec()
            .into_iter()
            .filter(|calendar| calendar.title().to_string() == requested_name)
            .collect::<Vec<_>>()
    };
    let alternatives = || compatible_calendar_names(store, required_availability);
    let calendar = match matching.as_slice() {
        [] => return Ok(false),
        [calendar] => calendar,
        _ => {
            return Err(target_failure(
                "calendar_name_ambiguous",
                "More than one calendar has the exact requested name.",
                requested_name,
                alternatives(),
            ))
        }
    };
    if !unsafe { calendar.allowsContentModifications() } {
        return Err(target_failure(
            "calendar_read_only",
            "The exact requested calendar exists but is read-only.",
            requested_name,
            alternatives(),
        ));
    }
    if !calendar_supports_availability(calendar, required_availability) {
        return Err(target_failure(
            "calendar_availability_unsupported",
            "The exact requested calendar cannot represent the event availability required by this task.",
            requested_name,
            alternatives(),
        ));
    }
    Ok(true)
}

fn remove_calendar_and_verify(store: &EKEventStore, calendar: &EKCalendar) -> bool {
    unsafe {
        let calendar_id = calendar.calendarIdentifier().to_string();
        let removed = store.removeCalendar_commit_error(calendar, true).is_ok();
        let absent_after_removal = calendar_id.is_empty() || {
            let identifier = NSString::from_str(&calendar_id);
            store.calendarWithIdentifier(&identifier).is_none()
        };
        removed && absent_after_removal
    }
}

fn stop_cancelled_event_mutation(
    trace: &CalendarOperationTrace,
    store: &EKEventStore,
    event: &EKEvent,
    saved_identifier: Option<&NSString>,
) -> Result<(), CalendarReadFailure> {
    if !trace.cancellation_requested() {
        return Ok(());
    }
    let removed = unsafe {
        store
            .removeEvent_span_error(event, EKSpan::ThisEvent)
            .is_ok()
    };
    let absent = saved_identifier
        .is_none_or(|identifier| unsafe { store.eventWithIdentifier(identifier).is_none() });
    let verified = removed && absent;
    trace.record_cancellation_cleanup(verified);
    if verified {
        Err(failure(
            "calendar_operation_cancelled",
            "Calendar removed the late event after the operation timed out.",
            true,
        ))
    } else {
        Err(failure(
            "calendar_event_cleanup_failed",
            "Calendar could not remove an event that finished after the operation timed out. Review Calendar before retrying.",
            false,
        ))
    }
}

fn stop_cancelled_calendar_mutation(
    trace: &CalendarOperationTrace,
    store: &EKEventStore,
    requested_name: &str,
    candidates: &[Retained<EKCalendar>],
) -> Result<(), CalendarReadFailure> {
    if !trace.cancellation_requested() {
        return Ok(());
    }
    let exact = candidates
        .iter()
        .filter(|calendar| unsafe { calendar.title().to_string() == requested_name })
        .collect::<Vec<_>>();
    let verified = match exact.as_slice() {
        [] => true,
        [calendar] => remove_calendar_and_verify(store, calendar),
        _ => false,
    };
    trace.record_cancellation_cleanup(verified);
    if verified {
        Err(failure(
            "calendar_operation_cancelled",
            "Calendar removed the late calendar after the operation timed out.",
            true,
        ))
    } else {
        Err(failure(
            "calendar_cleanup_failed",
            "Calendar could not remove a calendar that finished after the operation timed out. Review Calendar before retrying.",
            false,
        ))
    }
}

pub(super) fn remove_calendar_blocking(
    calendar_id: &str,
    trace: &CalendarOperationTrace,
) -> Result<(), CalendarReadFailure> {
    autoreleasepool(|_| unsafe {
        let store = EKEventStore::init(EKEventStore::alloc());
        require_current_full_calendar_access()?;
        let identifier = NSString::from_str(calendar_id);
        let Some(calendar) = store.calendarWithIdentifier(&identifier) else {
            return Ok(());
        };
        trace.require_not_cancelled()?;
        store
            .removeCalendar_commit_error(&calendar, true)
            .map_err(|_| {
                failure(
                    "calendar_cleanup_failed",
                    "The calendar created during recovery could not be removed.",
                    false,
                )
            })?;
        trace.record_cancellation_cleanup(trace.cancellation_requested());
        Ok(())
    })
}

pub(super) fn remove_calendar_event_blocking(
    event_id: &str,
    trace: &CalendarOperationTrace,
) -> Result<(), CalendarReadFailure> {
    autoreleasepool(|_| unsafe {
        let store = EKEventStore::init(EKEventStore::alloc());
        require_current_full_calendar_access()?;
        let identifier = NSString::from_str(event_id);
        let Some(event) = store.eventWithIdentifier(&identifier) else {
            return Ok(());
        };
        trace.require_not_cancelled()?;
        store
            .removeEvent_span_error(&event, EKSpan::ThisEvent)
            .map_err(|_| {
                failure(
                    "calendar_event_cleanup_failed",
                    "Calendar could not remove the event after its slot changed. Review Calendar before retrying.",
                    false,
                )
            })?;
        if store.eventWithIdentifier(&identifier).is_some() {
            return Err(failure(
                "calendar_event_cleanup_failed",
                "Calendar still contains the event after cleanup. Review Calendar before retrying.",
                false,
            ));
        }
        trace.record_cancellation_cleanup(trace.cancellation_requested());
        Ok(())
    })
}

fn native_availability(value: CalendarEventAvailability) -> EKEventAvailability {
    match value {
        CalendarEventAvailability::Busy => EKEventAvailability::Busy,
        CalendarEventAvailability::Free => EKEventAvailability::Free,
        CalendarEventAvailability::Tentative => EKEventAvailability::Tentative,
    }
}

fn availability_mask(value: CalendarEventAvailability) -> EKCalendarEventAvailabilityMask {
    match value {
        CalendarEventAvailability::Busy => EKCalendarEventAvailabilityMask::Busy,
        CalendarEventAvailability::Free => EKCalendarEventAvailabilityMask::Free,
        CalendarEventAvailability::Tentative => EKCalendarEventAvailabilityMask::Tentative,
    }
}

fn timestamp_millis(date: &NSDate) -> Result<i64, CalendarReadFailure> {
    let milliseconds = (date.timeIntervalSince1970() * 1_000.0).round();
    if !milliseconds.is_finite() || milliseconds < i64::MIN as f64 || milliseconds > i64::MAX as f64
    {
        return Err(failure(
            "calendar_event_verification_failed",
            "Calendar returned an invalid event date during verification.",
            true,
        ));
    }
    Ok(milliseconds as i64)
}

#[cfg(test)]
mod calendar_source_tests {
    use super::*;

    fn source(
        identifier: &str,
        title: &str,
        source_type: EKSourceType,
        is_delegate: bool,
        is_default: bool,
    ) -> CalendarSourceDescriptor {
        CalendarSourceDescriptor {
            identifier: identifier.to_string(),
            title: title.to_string(),
            source_type: source_type.0,
            is_delegate,
            is_default,
            supports_required_availability: true,
        }
    }

    #[test]
    fn calendar_source_selection_excludes_accounts_that_cannot_own_calendars() {
        assert!(!calendar_source_is_eligible(&source(
            "subscribed",
            "Subscribed",
            EKSourceType::Subscribed,
            false,
            false,
        )));
        assert!(!calendar_source_is_eligible(&source(
            "birthdays",
            "Birthdays",
            EKSourceType::Birthdays,
            false,
            false,
        )));
        assert!(!calendar_source_is_eligible(&source(
            "delegate",
            "Delegated",
            EKSourceType::CalDAV,
            true,
            false,
        )));
        assert!(calendar_source_is_eligible(&source(
            "local",
            "On My Mac",
            EKSourceType::Local,
            false,
            false,
        )));
    }

    #[test]
    fn calendar_source_selection_is_default_first_then_stable_by_capability() {
        let mut candidates = [
            source("exchange", "Work", EKSourceType::Exchange, false, false),
            source("caldav-z", "Zeta", EKSourceType::CalDAV, false, false),
            source("local", "On My Mac", EKSourceType::Local, false, false),
            source("caldav-a", "Alpha", EKSourceType::CalDAV, false, false),
            source(
                "default",
                "Default Work",
                EKSourceType::Exchange,
                false,
                true,
            ),
        ];
        candidates.sort_by_key(calendar_source_rank);
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.identifier.as_str())
                .collect::<Vec<_>>(),
            ["default", "local", "caldav-a", "caldav-z", "exchange"]
        );
    }

    #[test]
    fn calendar_creation_retries_only_source_capability_failures() {
        for code in [
            EKErrorCode::CalendarSourceCannotBeModified.0,
            EKErrorCode::SourceDoesNotAllowCalendarAddDelete.0,
            EKErrorCode::CalendarDoesNotAllowEvents.0,
            EKErrorCode::SourceDoesNotAllowEvents.0,
        ] {
            assert!(source_capability_error("EKErrorDomain", code));
        }
        assert!(!source_capability_error(
            "EKErrorDomain",
            EKErrorCode::EventStoreNotAuthorized.0,
        ));
        assert!(!source_capability_error(
            "NSCocoaErrorDomain",
            EKErrorCode::SourceDoesNotAllowCalendarAddDelete.0,
        ));
    }

    #[test]
    fn calendar_source_selection_prefers_event_compatible_accounts() {
        let mut incompatible_default = source(
            "caldav-default",
            "Default",
            EKSourceType::CalDAV,
            false,
            true,
        );
        incompatible_default.supports_required_availability = false;
        let compatible_exchange = source(
            "exchange-compatible",
            "Work",
            EKSourceType::Exchange,
            false,
            false,
        );
        let mut candidates = [incompatible_default, compatible_exchange];
        candidates.sort_by_key(calendar_source_rank);
        assert_eq!(candidates[0].identifier, "exchange-compatible");
    }
}
