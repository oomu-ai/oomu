use serde::Serialize;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AppControlErrorCode {
    DriverUnavailable,
    AccessibilityPermissionMissing,
    PermissionChanged,
    InvalidRequest,
    SessionNotFound,
    TaskBindingMismatch,
    BrowserRouteRequired,
    ObservationOnlyApplication,
    ApplicationNotAllowlisted,
    HiddenWindow,
    SecureField,
    AmbiguousTarget,
    StaleReference,
    CrossApplicationReference,
    FileScopeViolation,
    Unauthorized,
    NotRunning,
    PostconditionMismatch,
    UnexpectedNavigation,
    DriverFailure,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppControlError {
    pub code: AppControlErrorCode,
    pub message: String,
}

impl AppControlError {
    pub(crate) fn new(code: AppControlErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for AppControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl std::error::Error for AppControlError {}

pub type AppControlResult<T> = Result<T, AppControlError>;
