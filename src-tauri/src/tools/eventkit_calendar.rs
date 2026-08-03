use serde::{Deserialize, Serialize};

#[cfg(target_os = "macos")]
#[path = "eventkit_calendar/create.rs"]
mod create;

#[path = "eventkit_calendar/broker.rs"]
mod broker;

const MAXIMUM_EVENTS: usize = 200;
const MAXIMUM_CALENDAR_CHARACTERS: usize = 512;
const MAXIMUM_EVENT_TEXT_CHARACTERS: usize = 2_048;
const MAXIMUM_RECOVERY_CALENDARS: usize = 12;
const MAXIMUM_RECOVERY_CALENDAR_CHARACTERS: usize = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CalendarAuthorizationDisposition {
    FullAccess,
    NotDetermined,
    WriteOnly,
    Denied,
    Restricted,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CalendarOperationPhase {
    CheckingAccess,
    WaitingForPermission,
    ResettingStore,
    RefreshingSources,
    ReadingWindow,
    VerifyingResult,
    Writing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CalendarOperationOutcome {
    Succeeded,
    Failed,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CalendarOperationReceipt {
    pub(crate) operation_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) authorization_operation_id: Option<u64>,
    pub(crate) phase: CalendarOperationPhase,
    pub(crate) outcome: CalendarOperationOutcome,
    pub(crate) backend: String,
    pub(crate) elapsed_ms: u64,
    pub(crate) queue_ms: u64,
    pub(crate) authorization_before: CalendarAuthorizationDisposition,
    pub(crate) authorization_after: CalendarAuthorizationDisposition,
    pub(crate) permission_requested: bool,
    pub(crate) permission_granted: Option<bool>,
    pub(crate) native_error_code: Option<i64>,
    pub(crate) native_error_domain: Option<String>,
    pub(crate) store_reset: bool,
    pub(crate) sources_refreshed: bool,
    pub(crate) store_change_observed: bool,
    pub(crate) active_operation_count: usize,
    pub(crate) cancellation_requested: bool,
    pub(crate) cancellation_cleanup_verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) returned_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) matched_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) truncated: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CalendarFullAccessStatus {
    pub(crate) status: CalendarAuthorizationDisposition,
    pub(crate) full_access: bool,
    pub(crate) can_request_full_access: bool,
}

impl CalendarFullAccessStatus {
    fn from_disposition(status: CalendarAuthorizationDisposition) -> Self {
        Self {
            status,
            full_access: status == CalendarAuthorizationDisposition::FullAccess,
            can_request_full_access: matches!(
                status,
                CalendarAuthorizationDisposition::NotDetermined
                    | CalendarAuthorizationDisposition::WriteOnly
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CalendarReadRequest {
    pub(crate) calendar_name: String,
    pub(crate) start_timestamp: f64,
    pub(crate) end_timestamp: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CalendarEvent {
    pub(crate) event_id: String,
    pub(crate) calendar: String,
    pub(crate) name: String,
    pub(crate) start_time: String,
    pub(crate) end_time: String,
    pub(crate) location: String,
    pub(crate) notes: String,
    pub(crate) availability: CalendarEventAvailability,
    pub(crate) is_all_day: bool,
    pub(crate) declined_by_current_user: bool,
    pub(crate) time_zone: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CalendarWindow {
    pub(crate) start_date: String,
    pub(crate) end_date: String,
    pub(crate) time_zone: String,
}

#[derive(Debug, Clone)]
pub(crate) struct CalendarReadSuccess {
    pub(crate) calendar_name: String,
    pub(crate) window: CalendarWindow,
    pub(crate) events: Vec<CalendarEvent>,
    pub(crate) returned_count: usize,
    pub(crate) matched_count: usize,
    pub(crate) truncated: bool,
    pub(crate) receipt: Option<CalendarOperationReceipt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CalendarReadFailure {
    pub(crate) code: String,
    pub(crate) message: String,
    pub(crate) retryable: bool,
    pub(crate) requested_calendar_name: Option<String>,
    pub(crate) available_calendar_names: Vec<String>,
    pub(crate) receipt: Option<CalendarOperationReceipt>,
}

impl CalendarReadFailure {
    fn new(code: &str, message: &str, retryable: bool) -> Self {
        Self {
            code: code.to_string(),
            message: message.to_string(),
            retryable,
            requested_calendar_name: None,
            available_calendar_names: Vec::new(),
            receipt: None,
        }
    }

    fn with_receipt(mut self, receipt: CalendarOperationReceipt) -> Self {
        self.receipt = Some(receipt);
        self
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CalendarCreateRequest {
    pub(crate) calendar_name: String,
    pub(crate) title: String,
    pub(crate) start_timestamp_millis: i64,
    pub(crate) end_timestamp_millis: i64,
    pub(crate) start_date: String,
    pub(crate) end_date: String,
    pub(crate) location: String,
    pub(crate) notes: String,
    pub(crate) availability: CalendarEventAvailability,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum CalendarEventAvailability {
    Busy,
    Free,
    Tentative,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CalendarCreateSuccess {
    pub(crate) event_id: String,
    pub(crate) calendar_name: String,
    pub(crate) title: String,
    pub(crate) start_date: String,
    pub(crate) end_date: String,
    pub(crate) location: String,
    pub(crate) notes: String,
    pub(crate) availability: CalendarEventAvailability,
}

pub(crate) async fn read_calendar(
    request: CalendarReadRequest,
) -> Result<CalendarReadSuccess, CalendarReadFailure> {
    #[cfg(target_os = "macos")]
    {
        let authorization = ensure_full_calendar_access_with_receipt().await?;
        let (mut result, operation) =
            broker::run_serialized(broker::CalendarDeadlinePolicy::native(), move |trace| {
                read_calendar_blocking(request, None, &trace)
            })
            .await?;
        let mut receipt = merge_receipts(authorization, operation);
        attach_read_counts(&mut receipt, &result);
        result.receipt = Some(receipt);
        Ok(result)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = request;
        Err(failure(
            "calendar_unavailable",
            "Calendar is unavailable on this device.",
            false,
        ))
    }
}

pub(crate) async fn calendar_full_access_status() -> CalendarFullAccessStatus {
    #[cfg(target_os = "macos")]
    {
        tokio::task::spawn_blocking(current_calendar_full_access_status)
            .await
            .unwrap_or_else(|_| {
                CalendarFullAccessStatus::from_disposition(
                    CalendarAuthorizationDisposition::Unavailable,
                )
            })
    }

    #[cfg(not(target_os = "macos"))]
    {
        CalendarFullAccessStatus::from_disposition(CalendarAuthorizationDisposition::Unavailable)
    }
}

pub(crate) async fn ensure_full_calendar_access() -> Result<(), CalendarReadFailure> {
    #[cfg(target_os = "macos")]
    {
        ensure_full_calendar_access_with_receipt().await.map(drop)
    }

    #[cfg(not(target_os = "macos"))]
    {
        Err(failure(
            "calendar_permission_unavailable",
            "Calendar authorization is unavailable on this device.",
            false,
        ))
    }
}

#[cfg(target_os = "macos")]
pub(crate) async fn ensure_full_calendar_access_with_receipt(
) -> Result<CalendarOperationReceipt, CalendarReadFailure> {
    let (_, receipt) = broker::run_serialized(broker::CalendarDeadlinePolicy::native(), |trace| {
        objc2::rc::autoreleasepool(|_| {
            use objc2::AnyThread;
            use objc2_event_kit::EKEventStore;

            let store = unsafe { EKEventStore::init(EKEventStore::alloc()) };
            authorize_full_calendar_access(&store, &trace)
        })
    })
    .await?;
    Ok(receipt)
}

pub(crate) async fn read_calendar_requiring_unique_target(
    request: CalendarReadRequest,
    required_calendar_name: String,
    required_availability: CalendarEventAvailability,
) -> Result<CalendarReadSuccess, CalendarReadFailure> {
    #[cfg(target_os = "macos")]
    {
        let authorization = ensure_full_calendar_access_with_receipt().await?;
        let (mut result, operation) =
            broker::run_serialized(broker::CalendarDeadlinePolicy::native(), move |trace| {
                read_calendar_blocking(
                    request,
                    Some((required_calendar_name, required_availability)),
                    &trace,
                )
            })
            .await?;
        let mut receipt = merge_receipts(authorization, operation);
        attach_read_counts(&mut receipt, &result);
        result.receipt = Some(receipt);
        Ok(result)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (request, required_calendar_name, required_availability);
        Err(failure(
            "calendar_unavailable",
            "Calendar is unavailable on this device.",
            false,
        ))
    }
}

pub(crate) async fn create_calendar_event(
    request: CalendarCreateRequest,
) -> Result<CalendarCreateSuccess, CalendarReadFailure> {
    #[cfg(target_os = "macos")]
    {
        ensure_full_calendar_access_with_receipt().await?;
        broker::run_serialized(broker::CalendarDeadlinePolicy::native(), move |trace| {
            trace.set_phase(CalendarOperationPhase::Writing);
            create::create_calendar_event_blocking(request, &trace)
        })
        .await
        .map(|(value, _)| value)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = request;
        Err(failure(
            "calendar_unavailable",
            "Calendar is unavailable on this device.",
            false,
        ))
    }
}

pub(crate) async fn remove_calendar_event(event_id: String) -> Result<(), CalendarReadFailure> {
    #[cfg(target_os = "macos")]
    {
        ensure_full_calendar_access_with_receipt().await?;
        broker::run_serialized(broker::CalendarDeadlinePolicy::native(), move |trace| {
            trace.set_phase(CalendarOperationPhase::Writing);
            create::remove_calendar_event_blocking(&event_id, &trace)
        })
        .await
        .map(|(value, _)| value)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = event_id;
        Err(failure(
            "calendar_unavailable",
            "Calendar is unavailable on this device.",
            false,
        ))
    }
}

pub(crate) async fn create_calendar(
    calendar_name: String,
    required_availability: CalendarEventAvailability,
) -> Result<Option<String>, CalendarReadFailure> {
    #[cfg(target_os = "macos")]
    {
        ensure_full_calendar_access_with_receipt().await?;
        broker::run_serialized(broker::CalendarDeadlinePolicy::native(), move |trace| {
            trace.set_phase(CalendarOperationPhase::Writing);
            create::create_calendar_blocking(&calendar_name, required_availability, &trace)
        })
        .await
        .map(|(value, _)| value)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (calendar_name, required_availability);
        Err(failure(
            "calendar_unavailable",
            "Calendar is unavailable on this device.",
            false,
        ))
    }
}

pub(crate) async fn validate_calendar_target(
    calendar_name: String,
    required_availability: CalendarEventAvailability,
) -> Result<(), CalendarReadFailure> {
    #[cfg(target_os = "macos")]
    {
        ensure_full_calendar_access_with_receipt().await?;
        broker::run_serialized(broker::CalendarDeadlinePolicy::native(), move |trace| {
            trace.set_phase(CalendarOperationPhase::VerifyingResult);
            create::validate_calendar_target_blocking(&calendar_name, required_availability, &trace)
        })
        .await
        .map(|(value, _)| value)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (calendar_name, required_availability);
        Err(failure(
            "calendar_unavailable",
            "Calendar is unavailable on this device.",
            false,
        ))
    }
}

pub(crate) async fn compatible_calendar_names(
    required_availability: CalendarEventAvailability,
) -> Result<Vec<String>, CalendarReadFailure> {
    #[cfg(target_os = "macos")]
    {
        ensure_full_calendar_access_with_receipt().await?;
        broker::run_serialized(broker::CalendarDeadlinePolicy::native(), move |trace| {
            trace.set_phase(CalendarOperationPhase::ReadingWindow);
            create::compatible_calendar_names_blocking(required_availability, &trace)
        })
        .await
        .map(|(value, _)| value)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = required_availability;
        Err(failure(
            "calendar_unavailable",
            "Calendar is unavailable on this device.",
            false,
        ))
    }
}

pub(crate) async fn remove_calendar(calendar_id: String) -> Result<(), CalendarReadFailure> {
    #[cfg(target_os = "macos")]
    {
        ensure_full_calendar_access_with_receipt().await?;
        broker::run_serialized(broker::CalendarDeadlinePolicy::native(), move |trace| {
            trace.set_phase(CalendarOperationPhase::Writing);
            create::remove_calendar_blocking(&calendar_id, &trace)
        })
        .await
        .map(|(value, _)| value)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = calendar_id;
        Err(failure(
            "calendar_unavailable",
            "Calendar is unavailable on this device.",
            false,
        ))
    }
}

fn failure(code: &str, message: &str, retryable: bool) -> CalendarReadFailure {
    CalendarReadFailure::new(code, message, retryable)
}

fn target_failure(
    code: &str,
    message: &str,
    requested_calendar_name: &str,
    available_calendar_names: Vec<String>,
) -> CalendarReadFailure {
    CalendarReadFailure {
        code: code.to_string(),
        message: message.to_string(),
        retryable: false,
        requested_calendar_name: Some(bounded(
            requested_calendar_name.trim(),
            MAXIMUM_RECOVERY_CALENDAR_CHARACTERS,
        )),
        available_calendar_names,
        receipt: None,
    }
}

fn merge_receipts(
    authorization: CalendarOperationReceipt,
    mut operation: CalendarOperationReceipt,
) -> CalendarOperationReceipt {
    operation.authorization_operation_id = Some(authorization.operation_id);
    operation.elapsed_ms = operation
        .elapsed_ms
        .saturating_add(authorization.elapsed_ms);
    operation.queue_ms = operation.queue_ms.saturating_add(authorization.queue_ms);
    operation.authorization_before = authorization.authorization_before;
    operation.permission_requested = authorization.permission_requested;
    operation.permission_granted = authorization.permission_granted;
    operation.native_error_code = authorization.native_error_code;
    operation.native_error_domain = authorization.native_error_domain;
    operation.store_reset |= authorization.store_reset;
    operation.sources_refreshed |= authorization.sources_refreshed;
    operation.store_change_observed |= authorization.store_change_observed;
    operation
}

fn attach_read_counts(receipt: &mut CalendarOperationReceipt, result: &CalendarReadSuccess) {
    receipt.returned_count = Some(result.returned_count);
    receipt.matched_count = Some(result.matched_count);
    receipt.truncated = Some(result.truncated);
}

fn bounded_compatible_calendar_names<I>(calendars: I) -> Vec<String>
where
    I: IntoIterator<Item = (String, bool, bool)>,
{
    bounded_eligible_calendar_names(
        calendars
            .into_iter()
            .map(|(name, writable, compatible)| (name, writable && compatible)),
    )
}

fn bounded_eligible_calendar_names<I>(calendars: I) -> Vec<String>
where
    I: IntoIterator<Item = (String, bool)>,
{
    let mut counts = std::collections::BTreeMap::<String, (usize, usize)>::new();
    for (name, eligible) in calendars {
        let name = bounded(name.trim(), MAXIMUM_RECOVERY_CALENDAR_CHARACTERS);
        if !name.is_empty() {
            let entry = counts.entry(name).or_default();
            entry.0 += 1;
            entry.1 += usize::from(eligible);
        }
    }
    counts
        .into_iter()
        .filter_map(|(name, (total_count, eligible_count))| {
            (total_count == 1 && eligible_count == 1).then_some(name)
        })
        .take(MAXIMUM_RECOVERY_CALENDARS)
        .collect()
}

fn require_one_exact_calendar(matching_calendar_count: usize) -> Result<(), CalendarReadFailure> {
    match matching_calendar_count {
        1 => Ok(()),
        0 => Err(failure(
            "calendar_not_found",
            "The exact requested calendar was not found.",
            false,
        )),
        _ => Err(failure(
            "calendar_name_ambiguous",
            "More than one calendar has the exact requested name.",
            false,
        )),
    }
}

fn bounded(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}

fn authorization_failure(disposition: CalendarAuthorizationDisposition) -> CalendarReadFailure {
    match disposition {
        CalendarAuthorizationDisposition::Denied => failure(
            "calendar_permission_denied",
            "Calendar access is not authorized.",
            false,
        ),
        CalendarAuthorizationDisposition::Restricted => failure(
            "calendar_permission_restricted",
            "Calendar access is restricted by macOS policy.",
            false,
        ),
        CalendarAuthorizationDisposition::WriteOnly => failure(
            "calendar_permission_write_only",
            "Calendar access does not include permission to read events.",
            false,
        ),
        CalendarAuthorizationDisposition::NotDetermined
        | CalendarAuthorizationDisposition::Unavailable
        | CalendarAuthorizationDisposition::FullAccess => failure(
            "calendar_permission_unavailable",
            "Calendar authorization is unavailable.",
            false,
        ),
    }
}

fn authorization_requires_full_access_request(
    disposition: CalendarAuthorizationDisposition,
) -> Result<bool, CalendarReadFailure> {
    match disposition {
        CalendarAuthorizationDisposition::FullAccess => Ok(false),
        CalendarAuthorizationDisposition::NotDetermined
        | CalendarAuthorizationDisposition::WriteOnly => Ok(true),
        disposition => Err(authorization_failure(disposition)),
    }
}

fn verify_full_access_after_request(
    disposition: CalendarAuthorizationDisposition,
) -> Result<(), CalendarReadFailure> {
    if disposition == CalendarAuthorizationDisposition::FullAccess {
        Ok(())
    } else {
        Err(authorization_failure(disposition))
    }
}

#[cfg(target_os = "macos")]
fn require_current_full_calendar_access() -> Result<(), CalendarReadFailure> {
    verify_full_access_after_request(current_calendar_authorization_disposition())
}

#[cfg(target_os = "macos")]
fn require_active_calendar_read(
    trace: &broker::CalendarOperationTrace,
) -> Result<(), CalendarReadFailure> {
    match trace.require_not_cancelled() {
        Ok(()) => Ok(()),
        Err(error) => {
            trace.record_cancellation_cleanup(true);
            Err(error)
        }
    }
}

#[cfg(target_os = "macos")]
fn native_authorization_disposition(
    status: objc2_event_kit::EKAuthorizationStatus,
) -> CalendarAuthorizationDisposition {
    use objc2_event_kit::EKAuthorizationStatus;

    if status == EKAuthorizationStatus::FullAccess {
        CalendarAuthorizationDisposition::FullAccess
    } else if status == EKAuthorizationStatus::NotDetermined {
        CalendarAuthorizationDisposition::NotDetermined
    } else if status == EKAuthorizationStatus::WriteOnly {
        CalendarAuthorizationDisposition::WriteOnly
    } else if status == EKAuthorizationStatus::Denied {
        CalendarAuthorizationDisposition::Denied
    } else if status == EKAuthorizationStatus::Restricted {
        CalendarAuthorizationDisposition::Restricted
    } else {
        CalendarAuthorizationDisposition::Unavailable
    }
}

#[cfg(target_os = "macos")]
fn current_calendar_authorization_disposition() -> CalendarAuthorizationDisposition {
    use objc2_event_kit::{EKEntityType, EKEventStore};

    let status = unsafe { EKEventStore::authorizationStatusForEntityType(EKEntityType::Event) };
    let disposition = native_authorization_disposition(status);
    eprintln!(
        "OOMU_CALENDAR_AUTHORIZATION_STATUS raw={} disposition={disposition:?}",
        status.0
    );
    disposition
}

#[cfg(target_os = "macos")]
fn current_calendar_full_access_status() -> CalendarFullAccessStatus {
    CalendarFullAccessStatus::from_disposition(current_calendar_authorization_disposition())
}

#[cfg(target_os = "macos")]
fn ensure_calendar_store_change_observer() {
    use block2::RcBlock;
    use objc2_event_kit::EKEventStoreChangedNotification;
    use objc2_foundation::{NSNotification, NSNotificationCenter};
    use std::{ptr::NonNull, sync::Once};

    static OBSERVER: Once = Once::new();
    OBSERVER.call_once(|| unsafe {
        let center = NSNotificationCenter::defaultCenter();
        let callback = RcBlock::new(|_: NonNull<NSNotification>| broker::record_store_change());
        let observer = center.addObserverForName_object_queue_usingBlock(
            Some(EKEventStoreChangedNotification),
            None,
            None,
            &callback,
        );
        std::mem::forget(observer);
    });
}

#[cfg(target_os = "macos")]
fn authorize_full_calendar_access(
    store: &objc2_event_kit::EKEventStore,
    trace: &broker::CalendarOperationTrace,
) -> Result<(), CalendarReadFailure> {
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2::sel;
    use objc2_foundation::{NSError, NSObjectProtocol};
    use std::sync::mpsc;

    ensure_calendar_store_change_observer();
    trace.set_phase(CalendarOperationPhase::CheckingAccess);
    let initial = current_calendar_authorization_disposition();
    trace.record_authorization_before(initial);
    let previous = broker::remember_authorization(initial);
    let should_request = authorization_requires_full_access_request(initial)?;

    if should_request {
        if !store.respondsToSelector(sel!(requestFullAccessToEventsWithCompletion:)) {
            return Err(failure(
                "calendar_permission_unavailable",
                "Calendar full-access authorization is unavailable.",
                false,
            ));
        }

        trace.set_phase(CalendarOperationPhase::WaitingForPermission);
        let (sender, receiver) = mpsc::channel::<(bool, Option<i64>, Option<String>)>();
        let completion = RcBlock::new(move |granted: Bool, error: *mut NSError| {
            let (error_code, error_domain) = unsafe {
                error.as_ref().map_or((None, None), |error| {
                    (
                        Some(error.code() as i64),
                        Some(bounded(&error.domain().to_string(), 120)),
                    )
                })
            };
            let _ = sender.send((granted.as_bool(), error_code, error_domain));
        });
        unsafe {
            store.requestFullAccessToEventsWithCompletion(RcBlock::as_ptr(&completion));
        }
        let (granted, native_error_code, native_error_domain) = receiver.recv().map_err(|_| {
            failure(
                "calendar_authorization_interrupted",
                "Calendar could not finish the permission request.",
                true,
            )
        })?;
        trace.record_permission_callback(granted, native_error_code, native_error_domain.clone());
        if native_error_code.is_some() {
            return Err(failure(
                "calendar_authorization_failed",
                "Calendar could not finish the permission request.",
                true,
            ));
        }
    }

    let final_disposition = current_calendar_authorization_disposition();
    trace.record_authorization_after(final_disposition);
    broker::remember_authorization(final_disposition);
    verify_full_access_after_request(final_disposition)?;

    if broker::transition_requires_store_reset(previous, final_disposition, should_request) {
        trace.set_phase(CalendarOperationPhase::ResettingStore);
        unsafe { store.reset() };
        trace.record_store_reset();
    }
    trace.set_phase(CalendarOperationPhase::RefreshingSources);
    unsafe { store.refreshSourcesIfNecessary() };
    trace.record_sources_refreshed();
    Ok(())
}

#[cfg(target_os = "macos")]
fn formatted_event_date(
    date: &objc2_foundation::NSDate,
    time_zone: &objc2_foundation::NSTimeZone,
) -> Result<String, CalendarReadFailure> {
    use chrono::{DateTime, FixedOffset, SecondsFormat, Utc};

    let timestamp_millis = (date.timeIntervalSince1970() * 1_000.0).round();
    if !timestamp_millis.is_finite()
        || timestamp_millis < i64::MIN as f64
        || timestamp_millis > i64::MAX as f64
    {
        return Err(failure(
            "calendar_invalid_event",
            "Calendar returned an invalid event date.",
            true,
        ));
    }
    let utc = DateTime::<Utc>::from_timestamp_millis(timestamp_millis as i64).ok_or_else(|| {
        failure(
            "calendar_invalid_event",
            "Calendar returned an invalid event date.",
            true,
        )
    })?;
    let offset =
        FixedOffset::east_opt(time_zone.secondsFromGMTForDate(date) as i32).ok_or_else(|| {
            failure(
                "calendar_invalid_event",
                "Calendar returned an invalid event time zone.",
                true,
            )
        })?;
    Ok(utc
        .with_timezone(&offset)
        .to_rfc3339_opts(SecondsFormat::Secs, true))
}

#[cfg(target_os = "macos")]
unsafe fn validate_required_calendar_target(
    all_calendars: &objc2_foundation::NSArray<objc2_event_kit::EKCalendar>,
    required_calendar_target: Option<&(String, CalendarEventAvailability)>,
) -> Result<(), CalendarReadFailure> {
    let Some((required_calendar_name, required_availability)) = required_calendar_target else {
        return Ok(());
    };
    let required_calendar_name = required_calendar_name.trim();
    let native_calendars = all_calendars.to_vec();
    let matching_calendars = native_calendars
        .iter()
        .filter(|calendar| calendar.title().to_string() == required_calendar_name)
        .collect::<Vec<_>>();
    let compatible_names = || {
        bounded_compatible_calendar_names(native_calendars.iter().map(|calendar| {
            (
                calendar.title().to_string(),
                calendar.allowsContentModifications(),
                create::calendar_supports_availability(calendar, *required_availability),
            )
        }))
    };
    if let Err(failure) = require_one_exact_calendar(matching_calendars.len()) {
        return Err(target_failure(
            &failure.code,
            &failure.message,
            required_calendar_name,
            compatible_names(),
        ));
    }
    let calendar = matching_calendars[0];
    if !calendar.allowsContentModifications() {
        return Err(target_failure(
            "calendar_read_only",
            "The exact requested calendar is read-only.",
            required_calendar_name,
            compatible_names(),
        ));
    }
    if !create::calendar_supports_availability(calendar, *required_availability) {
        return Err(target_failure(
            "calendar_availability_unsupported",
            "The exact requested calendar cannot represent the event availability required by this task.",
            required_calendar_name,
            compatible_names(),
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
unsafe fn selected_calendars_for_read(
    all_calendars: &objc2_foundation::NSArray<objc2_event_kit::EKCalendar>,
    requested_calendar: &str,
) -> Result<Vec<objc2::rc::Retained<objc2_event_kit::EKCalendar>>, CalendarReadFailure> {
    let selected = all_calendars
        .to_vec()
        .into_iter()
        .filter(|calendar| {
            requested_calendar.is_empty()
                || calendar
                    .title()
                    .to_string()
                    .eq_ignore_ascii_case(requested_calendar)
        })
        .collect::<Vec<_>>();
    if !requested_calendar.is_empty() && selected.is_empty() {
        return Err(failure(
            "calendar_not_found",
            "The requested calendar was not found.",
            false,
        ));
    }
    Ok(selected)
}

#[cfg(target_os = "macos")]
unsafe fn calendar_event_rows(
    native_events: Vec<objc2::rc::Retained<objc2_event_kit::EKEvent>>,
    local_time_zone: &objc2_foundation::NSTimeZone,
    trace: &broker::CalendarOperationTrace,
) -> Result<Vec<(f64, CalendarEvent)>, CalendarReadFailure> {
    use objc2::Message;
    use objc2_event_kit::{EKEventAvailability, EKParticipantStatus};

    let mut rows = Vec::with_capacity(native_events.len());
    for event in native_events {
        require_active_calendar_read(trace)?;
        let start_date = event.startDate();
        let end_date = event.endDate();
        let event_time_zone = event.timeZone().unwrap_or_else(|| local_time_zone.retain());
        let calendar = event
            .calendar()
            .map(|calendar| calendar.title().to_string())
            .unwrap_or_default();
        rows.push((
            start_date.timeIntervalSince1970(),
            CalendarEvent {
                event_id: event
                    .eventIdentifier()
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                calendar: bounded(&calendar, MAXIMUM_CALENDAR_CHARACTERS),
                name: bounded(&event.title().to_string(), MAXIMUM_EVENT_TEXT_CHARACTERS),
                start_time: formatted_event_date(&start_date, &event_time_zone)?,
                end_time: formatted_event_date(&end_date, &event_time_zone)?,
                location: bounded(
                    &event
                        .location()
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                    MAXIMUM_EVENT_TEXT_CHARACTERS,
                ),
                notes: bounded(
                    &event
                        .notes()
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                    MAXIMUM_EVENT_TEXT_CHARACTERS,
                ),
                availability: match event.availability() {
                    EKEventAvailability::Free => CalendarEventAvailability::Free,
                    EKEventAvailability::Tentative => CalendarEventAvailability::Tentative,
                    _ => CalendarEventAvailability::Busy,
                },
                is_all_day: event.isAllDay(),
                declined_by_current_user: event.attendees().is_some_and(|attendees| {
                    attendees.to_vec().iter().any(|participant| {
                        participant.isCurrentUser()
                            && participant.participantStatus() == EKParticipantStatus::Declined
                    })
                }),
                time_zone: event_time_zone.name().to_string(),
            },
        ));
    }
    Ok(rows)
}

#[cfg(target_os = "macos")]
fn read_calendar_blocking(
    request: CalendarReadRequest,
    required_calendar_target: Option<(String, CalendarEventAvailability)>,
    trace: &broker::CalendarOperationTrace,
) -> Result<CalendarReadSuccess, CalendarReadFailure> {
    use objc2::rc::autoreleasepool;
    use objc2::AnyThread;
    use objc2_event_kit::{EKCalendar, EKEntityType, EKEventStore};
    use objc2_foundation::{NSArray, NSDate, NSTimeZone};

    autoreleasepool(|_| unsafe {
        ensure_calendar_store_change_observer();
        let store = EKEventStore::init(EKEventStore::alloc());
        require_active_calendar_read(trace)?;
        trace.set_phase(CalendarOperationPhase::CheckingAccess);
        let authorization = current_calendar_authorization_disposition();
        trace.record_authorization_before(authorization);
        trace.record_authorization_after(authorization);
        broker::remember_authorization(authorization);
        verify_full_access_after_request(authorization)?;
        trace.set_phase(CalendarOperationPhase::RefreshingSources);
        store.refreshSourcesIfNecessary();
        trace.record_sources_refreshed();
        require_active_calendar_read(trace)?;

        trace.set_phase(CalendarOperationPhase::ReadingWindow);
        require_active_calendar_read(trace)?;
        let all_calendars = store.calendarsForEntityType(EKEntityType::Event);
        require_active_calendar_read(trace)?;
        validate_required_calendar_target(&all_calendars, required_calendar_target.as_ref())?;
        let requested_calendar = request.calendar_name.trim();
        let selected_calendars = selected_calendars_for_read(&all_calendars, requested_calendar)?;

        let start_date = NSDate::dateWithTimeIntervalSince1970(request.start_timestamp);
        let end_date = NSDate::dateWithTimeIntervalSince1970(request.end_timestamp);
        let calendar_filter = (!selected_calendars.is_empty())
            .then(|| NSArray::<EKCalendar>::from_retained_slice(&selected_calendars));
        let predicate = store.predicateForEventsWithStartDate_endDate_calendars(
            &start_date,
            &end_date,
            calendar_filter.as_deref(),
        );
        require_active_calendar_read(trace)?;
        let matching_events = store.eventsMatchingPredicate(&predicate);
        require_active_calendar_read(trace)?;
        let local_time_zone = NSTimeZone::localTimeZone();
        let mut events = calendar_event_rows(matching_events.to_vec(), &local_time_zone, trace)?;
        require_active_calendar_read(trace)?;
        events.sort_by(|left, right| {
            left.0
                .total_cmp(&right.0)
                .then_with(|| left.1.name.cmp(&right.1.name))
        });

        let matched_count = events.len();
        let events = events
            .into_iter()
            .take(MAXIMUM_EVENTS)
            .map(|(_, event)| event)
            .collect::<Vec<_>>();
        let returned_count = events.len();
        let window_time_zone = NSTimeZone::localTimeZone();
        trace.set_phase(CalendarOperationPhase::VerifyingResult);
        require_active_calendar_read(trace)?;
        trace.verify_store_unchanged()?;
        Ok(CalendarReadSuccess {
            calendar_name: requested_calendar.to_string(),
            window: CalendarWindow {
                start_date: formatted_event_date(&start_date, &window_time_zone)?,
                end_date: formatted_event_date(&end_date, &window_time_zone)?,
                time_zone: window_time_zone.name().to_string(),
            },
            events,
            returned_count,
            matched_count,
            truncated: matched_count > returned_count,
            receipt: None,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_bounds_are_unicode_safe() {
        assert_eq!(bounded("Calendrier 🗓️", 11), "Calendrier ");
    }

    #[test]
    fn failures_are_typed_and_sanitized() {
        assert_eq!(
            failure("calendar_read_failed", "Calendar could not be read.", true),
            CalendarReadFailure {
                code: "calendar_read_failed".to_string(),
                message: "Calendar could not be read.".to_string(),
                retryable: true,
                requested_calendar_name: None,
                available_calendar_names: Vec::new(),
                receipt: None,
            }
        );
    }

    #[test]
    fn recovery_calendar_names_are_unique_writable_and_bounded() {
        let calendars = vec![
            ("Work".to_string(), true),
            ("Work".to_string(), true),
            ("Shared".to_string(), true),
            ("Shared".to_string(), false),
            ("Personal".to_string(), true),
            ("Read only".to_string(), false),
        ];
        assert_eq!(
            bounded_eligible_calendar_names(calendars),
            vec!["Personal".to_string()]
        );
    }

    #[test]
    fn recovery_calendar_names_require_unique_writable_availability_support() {
        let calendars = vec![
            ("Family".to_string(), true, false),
            ("Personal".to_string(), true, true),
            ("Read only".to_string(), false, true),
            ("Duplicate".to_string(), true, true),
            ("Duplicate".to_string(), false, false),
        ];
        assert_eq!(
            bounded_compatible_calendar_names(calendars),
            vec!["Personal".to_string()]
        );
    }

    #[test]
    fn unique_calendar_identity_requires_exactly_one_native_match() {
        assert!(require_one_exact_calendar(1).is_ok());
        assert_eq!(
            require_one_exact_calendar(0).unwrap_err().code,
            "calendar_not_found"
        );
        assert_eq!(
            require_one_exact_calendar(2).unwrap_err().code,
            "calendar_name_ambiguous"
        );
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn cancelled_read_records_verified_no_mutation_cleanup() {
        let failure = broker::run_serialized(broker::CalendarDeadlinePolicy::native(), |trace| {
            trace.cancel();
            require_active_calendar_read(&trace)
        })
        .await
        .expect_err("cancelled read stops");
        assert_eq!(failure.code, "calendar_operation_cancelled");
        assert!(
            failure
                .receipt
                .expect("cancel receipt exists")
                .cancellation_cleanup_verified
        );
    }

    #[test]
    fn authorization_status_disposition_is_complete() {
        let cases = [
            (CalendarAuthorizationDisposition::FullAccess, true, false),
            (CalendarAuthorizationDisposition::NotDetermined, false, true),
            (CalendarAuthorizationDisposition::WriteOnly, false, true),
            (CalendarAuthorizationDisposition::Denied, false, false),
            (CalendarAuthorizationDisposition::Restricted, false, false),
            (CalendarAuthorizationDisposition::Unavailable, false, false),
        ];
        for (disposition, full_access, can_request_full_access) in cases {
            let status = CalendarFullAccessStatus::from_disposition(disposition);
            assert_eq!(status.status, disposition);
            assert_eq!(status.full_access, full_access, "{disposition:?}");
            assert_eq!(
                status.can_request_full_access, can_request_full_access,
                "{disposition:?}"
            );
        }
    }

    #[test]
    fn not_determined_and_write_only_both_request_full_access() {
        for disposition in [
            CalendarAuthorizationDisposition::NotDetermined,
            CalendarAuthorizationDisposition::WriteOnly,
        ] {
            assert_eq!(
                authorization_requires_full_access_request(disposition),
                Ok(true),
                "{disposition:?}"
            );
        }
        assert_eq!(
            authorization_requires_full_access_request(
                CalendarAuthorizationDisposition::FullAccess
            ),
            Ok(false)
        );
    }

    #[test]
    fn post_request_authorization_is_rechecked_and_classified() {
        assert!(
            verify_full_access_after_request(CalendarAuthorizationDisposition::FullAccess).is_ok()
        );
        for (disposition, code) in [
            (
                CalendarAuthorizationDisposition::NotDetermined,
                "calendar_permission_unavailable",
            ),
            (
                CalendarAuthorizationDisposition::WriteOnly,
                "calendar_permission_write_only",
            ),
            (
                CalendarAuthorizationDisposition::Denied,
                "calendar_permission_denied",
            ),
            (
                CalendarAuthorizationDisposition::Restricted,
                "calendar_permission_restricted",
            ),
            (
                CalendarAuthorizationDisposition::Unavailable,
                "calendar_permission_unavailable",
            ),
        ] {
            assert_eq!(
                verify_full_access_after_request(disposition)
                    .expect_err("full access is required")
                    .code,
                code,
                "{disposition:?}"
            );
        }
    }
}
