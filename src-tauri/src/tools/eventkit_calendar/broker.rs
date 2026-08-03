use super::{
    CalendarAuthorizationDisposition, CalendarOperationOutcome, CalendarOperationPhase,
    CalendarOperationReceipt, CalendarReadFailure,
};
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    Arc, Mutex, OnceLock,
};
use std::time::{Duration, Instant};
use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};

const CALENDAR_QUEUE_WAIT: Duration = Duration::from_secs(30);
const ACCESS_CHECK_DEADLINE: Duration = Duration::from_secs(5);
const STORE_RESET_DEADLINE: Duration = Duration::from_secs(10);
const SOURCE_REFRESH_DEADLINE: Duration = Duration::from_secs(10);
const EVENT_FETCH_DEADLINE: Duration = Duration::from_secs(30);
const RESULT_VERIFICATION_DEADLINE: Duration = Duration::from_secs(5);
const WRITE_DEADLINE: Duration = Duration::from_secs(30);

static CALENDAR_GATE: OnceLock<Arc<Semaphore>> = OnceLock::new();
static NEXT_OPERATION_ID: AtomicU64 = AtomicU64::new(1);
static ACTIVE_OPERATIONS: AtomicUsize = AtomicUsize::new(0);
static STORE_CHANGE_EPOCH: AtomicU64 = AtomicU64::new(0);
static LAST_AUTHORIZATION: OnceLock<Mutex<Option<CalendarAuthorizationDisposition>>> =
    OnceLock::new();

fn gate() -> Arc<Semaphore> {
    CALENDAR_GATE
        .get_or_init(|| Arc::new(Semaphore::new(1)))
        .clone()
}

fn authorization_memory() -> &'static Mutex<Option<CalendarAuthorizationDisposition>> {
    LAST_AUTHORIZATION.get_or_init(|| Mutex::new(None))
}

#[derive(Debug)]
struct TraceState {
    operation_id: u64,
    phase: CalendarOperationPhase,
    started: Instant,
    queue_ms: u64,
    authorization_before: CalendarAuthorizationDisposition,
    authorization_after: CalendarAuthorizationDisposition,
    permission_requested: bool,
    permission_granted: Option<bool>,
    native_error_code: Option<i64>,
    native_error_domain: Option<String>,
    store_reset: bool,
    sources_refreshed: bool,
    store_change_epoch: u64,
    store_change_observed: bool,
    active_operation_count: usize,
    cancellation_cleanup_verified: bool,
}

#[derive(Clone, Debug)]
pub(super) struct CalendarOperationTrace {
    state: Arc<Mutex<TraceState>>,
    phase_sender: watch::Sender<CalendarOperationPhase>,
    cancellation_requested: Arc<AtomicBool>,
}

impl CalendarOperationTrace {
    fn new(operation_id: u64, queue_ms: u64) -> Self {
        let (phase_sender, _) = watch::channel(CalendarOperationPhase::CheckingAccess);
        Self {
            state: Arc::new(Mutex::new(TraceState {
                operation_id,
                phase: CalendarOperationPhase::CheckingAccess,
                started: Instant::now(),
                queue_ms,
                authorization_before: CalendarAuthorizationDisposition::Unavailable,
                authorization_after: CalendarAuthorizationDisposition::Unavailable,
                permission_requested: false,
                permission_granted: None,
                native_error_code: None,
                native_error_domain: None,
                store_reset: false,
                sources_refreshed: false,
                store_change_epoch: STORE_CHANGE_EPOCH.load(Ordering::SeqCst),
                store_change_observed: false,
                active_operation_count: 0,
                cancellation_cleanup_verified: false,
            })),
            phase_sender,
            cancellation_requested: Arc::new(AtomicBool::new(false)),
        }
    }

    fn update(&self, apply: impl FnOnce(&mut TraceState)) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        apply(&mut state);
    }

    pub(super) fn set_phase(&self, phase: CalendarOperationPhase) {
        self.update(|state| state.phase = phase);
        self.phase_sender.send_replace(phase);
        self.emit_phase();
    }

    pub(super) fn cancel(&self) {
        self.cancellation_requested.store(true, Ordering::SeqCst);
    }

    pub(super) fn cancellation_requested(&self) -> bool {
        self.cancellation_requested.load(Ordering::SeqCst)
    }

    pub(super) fn require_not_cancelled(&self) -> Result<(), CalendarReadFailure> {
        if self.cancellation_requested() {
            Err(CalendarReadFailure::new(
                "calendar_operation_cancelled",
                "Calendar stopped the operation before it changed anything.",
                true,
            ))
        } else {
            Ok(())
        }
    }

    pub(super) fn record_cancellation_cleanup(&self, verified: bool) {
        self.update(|state| state.cancellation_cleanup_verified = verified);
    }

    fn subscribe(&self) -> watch::Receiver<CalendarOperationPhase> {
        self.phase_sender.subscribe()
    }

    pub(super) fn record_authorization_before(
        &self,
        disposition: CalendarAuthorizationDisposition,
    ) {
        self.update(|state| {
            state.authorization_before = disposition;
            state.authorization_after = disposition;
        });
    }

    pub(super) fn record_authorization_after(&self, disposition: CalendarAuthorizationDisposition) {
        self.update(|state| state.authorization_after = disposition);
    }

    pub(super) fn record_permission_callback(
        &self,
        granted: bool,
        native_error_code: Option<i64>,
        native_error_domain: Option<String>,
    ) {
        self.update(|state| {
            state.permission_requested = true;
            state.permission_granted = Some(granted);
            state.native_error_code = native_error_code;
            state.native_error_domain = native_error_domain;
        });
    }

    pub(super) fn record_store_reset(&self) {
        self.update(|state| state.store_reset = true);
    }

    pub(super) fn record_sources_refreshed(&self) {
        self.update(|state| state.sources_refreshed = true);
    }

    pub(super) fn verify_store_unchanged(&self) -> Result<(), CalendarReadFailure> {
        let current_epoch = STORE_CHANGE_EPOCH.load(Ordering::SeqCst);
        let changed = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            let changed = current_epoch != state.store_change_epoch;
            state.store_change_observed = changed;
            changed
        };
        if changed {
            Err(CalendarReadFailure::new(
                "calendar_store_changed",
                "Calendar changed while OOMU was reading it.",
                true,
            ))
        } else {
            Ok(())
        }
    }

    fn record_active_operation_count(&self, active_operation_count: usize) {
        self.update(|state| state.active_operation_count = active_operation_count);
        self.emit_phase();
    }

    pub(super) fn receipt(&self, outcome: CalendarOperationOutcome) -> CalendarOperationReceipt {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        CalendarOperationReceipt {
            operation_id: state.operation_id,
            authorization_operation_id: None,
            phase: state.phase,
            outcome,
            backend: "eventkit".to_string(),
            elapsed_ms: elapsed_ms(state.started.elapsed()),
            queue_ms: state.queue_ms,
            authorization_before: state.authorization_before,
            authorization_after: state.authorization_after,
            permission_requested: state.permission_requested,
            permission_granted: state.permission_granted,
            native_error_code: state.native_error_code,
            native_error_domain: state.native_error_domain.clone(),
            store_reset: state.store_reset,
            sources_refreshed: state.sources_refreshed,
            store_change_observed: state.store_change_observed,
            active_operation_count: state.active_operation_count,
            cancellation_requested: self.cancellation_requested(),
            cancellation_cleanup_verified: state.cancellation_cleanup_verified,
            returned_count: None,
            matched_count: None,
            truncated: None,
        }
    }

    fn emit_phase(&self) {
        if !crate::diagnostic_output::native_acceptance_enabled() {
            return;
        }
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let evidence = CalendarPhaseEvidence {
            operation_id: state.operation_id,
            phase: state.phase,
            elapsed_ms: elapsed_ms(state.started.elapsed()),
            active_operation_count: state.active_operation_count,
            cancellation_requested: self.cancellation_requested(),
        };
        if let Ok(encoded) = serde_json::to_string(&evidence) {
            eprintln!("OOMU_CALENDAR_PHASE_RECEIPT {encoded}");
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CalendarPhaseEvidence {
    operation_id: u64,
    phase: CalendarOperationPhase,
    elapsed_ms: u64,
    active_operation_count: usize,
    cancellation_requested: bool,
}

pub(super) fn record_store_change() {
    STORE_CHANGE_EPOCH.fetch_add(1, Ordering::SeqCst);
}

#[derive(Clone, Copy)]
pub(super) struct CalendarDeadlinePolicy {
    access_check: Duration,
    store_reset: Duration,
    source_refresh: Duration,
    event_fetch: Duration,
    result_verification: Duration,
    write: Duration,
}

impl CalendarDeadlinePolicy {
    pub(super) const fn native() -> Self {
        Self {
            access_check: ACCESS_CHECK_DEADLINE,
            store_reset: STORE_RESET_DEADLINE,
            source_refresh: SOURCE_REFRESH_DEADLINE,
            event_fetch: EVENT_FETCH_DEADLINE,
            result_verification: RESULT_VERIFICATION_DEADLINE,
            write: WRITE_DEADLINE,
        }
    }

    #[cfg(test)]
    const fn testing(deadline: Duration) -> Self {
        Self {
            access_check: deadline,
            store_reset: deadline,
            source_refresh: deadline,
            event_fetch: deadline,
            result_verification: deadline,
            write: deadline,
        }
    }

    fn deadline(self, phase: CalendarOperationPhase) -> Option<Duration> {
        match phase {
            CalendarOperationPhase::CheckingAccess => Some(self.access_check),
            CalendarOperationPhase::WaitingForPermission => None,
            CalendarOperationPhase::ResettingStore => Some(self.store_reset),
            CalendarOperationPhase::RefreshingSources => Some(self.source_refresh),
            CalendarOperationPhase::ReadingWindow => Some(self.event_fetch),
            CalendarOperationPhase::VerifyingResult => Some(self.result_verification),
            CalendarOperationPhase::Writing => Some(self.write),
        }
    }
}

struct ActiveOperation {
    _permit: OwnedSemaphorePermit,
}

impl ActiveOperation {
    fn new(permit: OwnedSemaphorePermit) -> Self {
        let previous = ACTIVE_OPERATIONS.fetch_add(1, Ordering::SeqCst);
        debug_assert_eq!(previous, 0, "Calendar broker admitted overlapping work");
        Self { _permit: permit }
    }
}

impl Drop for ActiveOperation {
    fn drop(&mut self) {
        ACTIVE_OPERATIONS.fetch_sub(1, Ordering::SeqCst);
    }
}

pub(super) async fn run_serialized<T, F>(
    deadline_policy: CalendarDeadlinePolicy,
    operation: F,
) -> Result<(T, CalendarOperationReceipt), CalendarReadFailure>
where
    T: Send + 'static,
    F: FnOnce(CalendarOperationTrace) -> Result<T, CalendarReadFailure> + Send + 'static,
{
    let queued_at = Instant::now();
    let permit = tokio::time::timeout(CALENDAR_QUEUE_WAIT, gate().acquire_owned())
        .await
        .map_err(|_| {
            CalendarReadFailure::new(
                "calendar_operation_busy",
                "Calendar is still finishing an earlier request.",
                true,
            )
        })?
        .map_err(|_| {
            CalendarReadFailure::new(
                "calendar_operation_unavailable",
                "Calendar is unavailable.",
                true,
            )
        })?;
    let operation_id = NEXT_OPERATION_ID.fetch_add(1, Ordering::Relaxed);
    let trace = CalendarOperationTrace::new(operation_id, elapsed_ms(queued_at.elapsed()));
    let mut phase_updates = trace.subscribe();
    let worker_trace = trace.clone();
    let mut worker = Box::pin(tokio::task::spawn_blocking(move || {
        let _active = ActiveOperation::new(permit);
        worker_trace.record_active_operation_count(ACTIVE_OPERATIONS.load(Ordering::SeqCst));
        let result = operation(worker_trace.clone());
        if worker_trace.cancellation_requested() {
            let outcome = if result.is_ok() {
                CalendarOperationOutcome::Succeeded
            } else {
                CalendarOperationOutcome::Failed
            };
            if let Ok(encoded) = serde_json::to_string(&worker_trace.receipt(outcome)) {
                eprintln!("OOMU_CALENDAR_LATE_COMPLETION_RECEIPT {encoded}");
            }
        }
        result
    }));

    let completed = loop {
        let phase = *phase_updates.borrow_and_update();
        let phase_changed = phase_updates.changed();
        if let Some(deadline) = deadline_policy.deadline(phase) {
            tokio::select! {
                biased;
                completed = &mut worker => break completed,
                changed = phase_changed => {
                    if changed.is_err() {
                        break worker.await;
                    }
                }
                _ = tokio::time::sleep(deadline) => {
                    trace.cancel();
                    let receipt = trace.receipt(CalendarOperationOutcome::TimedOut);
                    return Err(timeout_failure(phase).with_receipt(receipt));
                }
            }
        } else {
            tokio::select! {
                biased;
                completed = &mut worker => break completed,
                changed = phase_changed => {
                    if changed.is_err() {
                        break worker.await;
                    }
                }
            }
        }
    };

    match completed {
        Ok(Ok(value)) => Ok((value, trace.receipt(CalendarOperationOutcome::Succeeded))),
        Ok(Err(failure)) => {
            Err(failure.with_receipt(trace.receipt(CalendarOperationOutcome::Failed)))
        }
        Err(error) => {
            eprintln!("OOMU_CALENDAR_WORKER_JOIN_FAILED operation_id={operation_id} error={error}");
            Err(CalendarReadFailure::new(
                "calendar_operation_interrupted",
                "Calendar could not finish this request.",
                true,
            )
            .with_receipt(trace.receipt(CalendarOperationOutcome::Failed)))
        }
    }
}

fn timeout_failure(phase: CalendarOperationPhase) -> CalendarReadFailure {
    let (code, message) = match phase {
        CalendarOperationPhase::CheckingAccess => (
            "calendar_access_check_timeout",
            "Calendar took too long to check access.",
        ),
        CalendarOperationPhase::WaitingForPermission => (
            "calendar_authorization_timeout",
            "Calendar is still waiting for a permission decision.",
        ),
        CalendarOperationPhase::ResettingStore => (
            "calendar_store_reset_timeout",
            "Calendar took too long to apply the new permission.",
        ),
        CalendarOperationPhase::RefreshingSources => (
            "calendar_source_refresh_timeout",
            "Calendar took too long to refresh.",
        ),
        CalendarOperationPhase::ReadingWindow => {
            ("calendar_read_timeout", "Calendar did not respond in time.")
        }
        CalendarOperationPhase::VerifyingResult => (
            "calendar_result_verification_timeout",
            "Calendar took too long to verify the result.",
        ),
        CalendarOperationPhase::Writing => (
            "calendar_write_timeout",
            "Calendar took too long to finish the approved change.",
        ),
    };
    CalendarReadFailure::new(code, message, true)
}

pub(super) fn remember_authorization(
    current: CalendarAuthorizationDisposition,
) -> Option<CalendarAuthorizationDisposition> {
    let mut previous = authorization_memory()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let observed = *previous;
    *previous = Some(current);
    observed
}

pub(super) fn transition_requires_store_reset(
    previous: Option<CalendarAuthorizationDisposition>,
    current: CalendarAuthorizationDisposition,
    permission_requested: bool,
) -> bool {
    current == CalendarAuthorizationDisposition::FullAccess
        && (permission_requested
            || previous.is_some_and(|value| value != CalendarAuthorizationDisposition::FullAccess))
}

fn elapsed_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn calendar_store_resets_after_access_transition() {
        assert!(transition_requires_store_reset(
            Some(CalendarAuthorizationDisposition::Denied),
            CalendarAuthorizationDisposition::FullAccess,
            false,
        ));
        assert!(transition_requires_store_reset(
            Some(CalendarAuthorizationDisposition::NotDetermined),
            CalendarAuthorizationDisposition::FullAccess,
            true,
        ));
        assert!(!transition_requires_store_reset(
            Some(CalendarAuthorizationDisposition::FullAccess),
            CalendarAuthorizationDisposition::FullAccess,
            false,
        ));
    }

    #[test]
    fn calendar_store_change_invalidates_inflight_read() {
        let trace = CalendarOperationTrace::new(1, 0);
        record_store_change();
        let failure = trace.verify_store_unchanged().unwrap_err();
        assert_eq!(failure.code, "calendar_store_changed");
        assert!(failure.retryable);
        assert!(
            trace
                .receipt(CalendarOperationOutcome::Failed)
                .store_change_observed
        );
    }

    #[tokio::test]
    async fn calendar_single_flight_blocks_overlapping_retry() {
        static PEAK: AtomicUsize = AtomicUsize::new(0);
        static CURRENT: AtomicUsize = AtomicUsize::new(0);
        let run = || async {
            run_serialized(
                CalendarDeadlinePolicy::testing(Duration::from_secs(1)),
                |_| {
                    let current = CURRENT.fetch_add(1, Ordering::SeqCst) + 1;
                    PEAK.fetch_max(current, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(25));
                    CURRENT.fetch_sub(1, Ordering::SeqCst);
                    Ok(())
                },
            )
            .await
        };
        let (left, right) = tokio::join!(run(), run());
        left.unwrap();
        right.unwrap();
        assert_eq!(PEAK.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn calendar_permission_wait_is_outside_read_deadline() {
        let (_, receipt) = run_serialized(
            CalendarDeadlinePolicy::testing(Duration::from_millis(5)),
            |trace| {
                trace.set_phase(CalendarOperationPhase::WaitingForPermission);
                std::thread::sleep(Duration::from_millis(20));
                Ok(())
            },
        )
        .await
        .unwrap();
        assert_eq!(receipt.outcome, CalendarOperationOutcome::Succeeded);
        assert_eq!(receipt.phase, CalendarOperationPhase::WaitingForPermission);
        assert!(receipt.elapsed_ms >= 15);
    }

    #[tokio::test]
    async fn calendar_phase_timeout_preserves_exact_refresh_boundary() {
        let failure = run_serialized(
            CalendarDeadlinePolicy::testing(Duration::from_millis(5)),
            |trace| {
                trace.set_phase(CalendarOperationPhase::RefreshingSources);
                std::thread::sleep(Duration::from_millis(30));
                Ok(())
            },
        )
        .await
        .unwrap_err();
        assert_eq!(failure.code, "calendar_source_refresh_timeout");
        let receipt = failure.receipt.expect("timeout receipt");
        assert_eq!(receipt.phase, CalendarOperationPhase::RefreshingSources);
        assert_eq!(receipt.outcome, CalendarOperationOutcome::TimedOut);
    }

    #[tokio::test]
    async fn timed_out_native_work_keeps_its_single_flight_lease() {
        let worker_finished = Arc::new(AtomicBool::new(false));
        let finished_by_worker = worker_finished.clone();
        let first = run_serialized(
            CalendarDeadlinePolicy::testing(Duration::from_millis(5)),
            move |_| {
                std::thread::sleep(Duration::from_millis(40));
                finished_by_worker.store(true, Ordering::SeqCst);
                Ok(())
            },
        )
        .await;
        assert_eq!(first.unwrap_err().code, "calendar_access_check_timeout");
        run_serialized(
            CalendarDeadlinePolicy::testing(Duration::from_secs(1)),
            move |_| {
                assert!(worker_finished.load(Ordering::SeqCst));
                Ok(())
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn timed_out_write_is_cancelled_cleaned_and_never_overlaps_retry() {
        let committed = Arc::new(AtomicUsize::new(0));
        let cleanup_verified = Arc::new(AtomicBool::new(false));
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let committed_for_worker = committed.clone();
        let cleanup_for_worker = cleanup_verified.clone();
        let active_for_worker = active.clone();
        let peak_for_worker = peak.clone();
        let first = run_serialized(
            CalendarDeadlinePolicy::testing(Duration::from_millis(5)),
            move |trace| {
                let current = active_for_worker.fetch_add(1, Ordering::SeqCst) + 1;
                peak_for_worker.fetch_max(current, Ordering::SeqCst);
                trace.set_phase(CalendarOperationPhase::Writing);
                trace.require_not_cancelled()?;
                std::thread::sleep(Duration::from_millis(25));
                committed_for_worker.fetch_add(1, Ordering::SeqCst);
                active_for_worker.fetch_sub(1, Ordering::SeqCst);
                if trace.cancellation_requested() {
                    committed_for_worker.fetch_sub(1, Ordering::SeqCst);
                    cleanup_for_worker.store(true, Ordering::SeqCst);
                    trace.record_cancellation_cleanup(true);
                    return Err(CalendarReadFailure::new(
                        "calendar_operation_cancelled",
                        "late write removed",
                        true,
                    ));
                }
                Ok(())
            },
        )
        .await;
        let timeout = first.unwrap_err();
        assert_eq!(timeout.code, "calendar_write_timeout");
        assert!(
            timeout
                .receipt
                .expect("timeout receipt")
                .cancellation_requested
        );

        let active_for_retry = active.clone();
        let peak_for_retry = peak.clone();
        run_serialized(
            CalendarDeadlinePolicy::testing(Duration::from_secs(1)),
            move |_| {
                let current = active_for_retry.fetch_add(1, Ordering::SeqCst) + 1;
                peak_for_retry.fetch_max(current, Ordering::SeqCst);
                active_for_retry.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .await
        .unwrap();
        assert_eq!(peak.load(Ordering::SeqCst), 1);
        assert_eq!(committed.load(Ordering::SeqCst), 0);
        assert!(cleanup_verified.load(Ordering::SeqCst));
    }
}
