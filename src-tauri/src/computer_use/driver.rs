use super::{
    contracts::{
        AccessibilityPermission, DesktopActionKind, DesktopApplicationObservation,
        DesktopSemanticAction, DesktopWindowObservation, ElementGeometry, ExpectedOutcomeKind,
        ObservedPostcondition, RedactedScreenshotReceipt,
    },
    error::{AppControlError, AppControlErrorCode, AppControlResult},
};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

#[derive(Clone, Debug)]
pub struct DriverCancellationToken {
    epoch: Arc<AtomicU64>,
    expected: u64,
    physical_input_epoch: Option<(Arc<AtomicU64>, u64)>,
}

impl DriverCancellationToken {
    pub(crate) fn new(
        epoch: Arc<AtomicU64>,
        expected: u64,
        physical_input_epoch: Option<(Arc<AtomicU64>, u64)>,
    ) -> Self {
        Self {
            epoch,
            expected,
            physical_input_epoch,
        }
    }

    pub fn cancelled(&self) -> bool {
        self.epoch.load(Ordering::SeqCst) != self.expected
            || self
                .physical_input_epoch
                .as_ref()
                .is_some_and(|(epoch, expected)| epoch.load(Ordering::SeqCst) != *expected)
    }
}

#[derive(Clone, Debug)]
pub struct DriverObservationRequest {
    pub session_id: String,
    pub project_id: String,
    pub task_run_id: String,
}

#[derive(Clone, Debug)]
pub struct DriverElement {
    pub element_key: String,
    pub role: String,
    pub label: Option<String>,
    pub value_digest: Option<String>,
    pub secure: bool,
    pub visible: bool,
    pub enabled: bool,
    pub in_modal: bool,
    pub supported_actions: Vec<DesktopActionKind>,
    pub geometry: Option<ElementGeometry>,
}

#[derive(Clone, Debug)]
pub struct DriverObservation {
    pub permission: AccessibilityPermission,
    pub application: DesktopApplicationObservation,
    pub window: DesktopWindowObservation,
    pub focused_element_key: Option<String>,
    pub elements: Vec<DriverElement>,
    pub screenshot: Option<RedactedScreenshotReceipt>,
}

#[derive(Clone, Debug)]
pub struct ResolvedDriverTarget {
    pub element_key: String,
    pub role: String,
    pub secure: bool,
    pub in_modal: bool,
    pub geometry: Option<ElementGeometry>,
}

#[derive(Clone, Debug)]
pub struct DriverActionRequest {
    pub session_id: String,
    pub project_id: String,
    pub task_run_id: String,
    pub application: DesktopApplicationObservation,
    pub window: DesktopWindowObservation,
    pub action: DesktopSemanticAction,
    pub expected_outcome: ExpectedOutcomeKind,
    pub targets: Vec<ResolvedDriverTarget>,
    pub selected_file: Option<PathBuf>,
    pub cancellation: DriverCancellationToken,
}

#[derive(Clone, Debug)]
pub struct DriverActionResult {
    pub receipt_token: String,
    pub postcondition: ObservedPostcondition,
}

/// Bounded native seam. Implementations receive semantic actions and resolved
/// accessibility targets only; there is no raw script or coordinate entrypoint.
/// An implementation must check `cancellation` immediately before commitment.
pub trait DesktopDriver: Send + Sync {
    fn observe(&self, request: &DriverObservationRequest) -> AppControlResult<DriverObservation>;

    fn perform(&self, request: &DriverActionRequest) -> AppControlResult<DriverActionResult>;
}

#[derive(Default)]
pub struct UnavailableDesktopDriver;

impl DesktopDriver for UnavailableDesktopDriver {
    fn observe(&self, _request: &DriverObservationRequest) -> AppControlResult<DriverObservation> {
        Err(AppControlError::new(
            AppControlErrorCode::DriverUnavailable,
            "App control is unavailable until the native accessibility driver is installed.",
        ))
    }

    fn perform(&self, _request: &DriverActionRequest) -> AppControlResult<DriverActionResult> {
        Err(AppControlError::new(
            AppControlErrorCode::DriverUnavailable,
            "App control cannot perform actions without the native accessibility driver.",
        ))
    }
}
