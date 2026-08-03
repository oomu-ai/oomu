mod commands;
mod contracts;
mod driver;
mod error;
mod evidence;
mod execution;
#[cfg(target_os = "macos")]
mod macos_actions;
#[cfg(target_os = "macos")]
mod macos_apple_event;
#[cfg(target_os = "macos")]
mod macos_driver;
#[cfg(target_os = "macos")]
mod macos_geometry;
#[cfg(target_os = "macos")]
mod macos_input;
#[cfg(target_os = "macos")]
mod macos_postcondition;
#[cfg(target_os = "macos")]
mod macos_process;
mod manager;
mod observation;
mod policy;
mod references;
mod session;
mod state;
mod task_tool;
mod verification;

#[cfg(test)]
mod action_adapter_tests;

pub use contracts::{
    AppControlApplicationView, AppControlControl, AppControlOutcomeStatus, AppControlOutcomeView,
    AppControlPauseReason, AppControlSessionRequest, AppControlSessionView, AppControlState,
    ControlAppControlSessionRequest, DesktopActionKind, DesktopActionOutcome, DesktopObservation,
    DesktopOutcomeReceipt, DesktopSemanticAction, ExecuteDesktopActionRequest, ExpectedOutcomeKind,
    GetAppControlStatusRequest, StartAppControlSession, StartAppControlSessionRequest,
};
pub use driver::{DesktopDriver, UnavailableDesktopDriver};
pub use error::{AppControlError, AppControlErrorCode, AppControlResult};
#[cfg(target_os = "macos")]
pub use macos_driver::MacOsAccessibilityDriver;
pub use manager::{AppControlManager, AppControlTimeSource, SystemAppControlTimeSource};
pub use policy::{
    AuthorityDecision, AuthorityRequest, DenyAllDesktopAuthority, DesktopApprovalBinding,
    DesktopAuthorityEvaluator, ReviewedScopeDesktopAuthority,
};
pub(crate) use task_tool::register_task_tool;

#[tauri::command]
pub fn get_app_control_status(
    request: GetAppControlStatusRequest,
    manager: tauri::State<'_, AppControlManager>,
) -> Result<Option<AppControlSessionView>, AppControlError> {
    commands::get_app_control_status_impl(request, manager)
}

#[tauri::command]
pub fn control_app_control_session(
    request: ControlAppControlSessionRequest,
    manager: tauri::State<'_, AppControlManager>,
) -> Result<AppControlSessionView, AppControlError> {
    commands::control_app_control_session_impl(request, manager)
}

#[tauri::command]
pub fn start_app_control_session(
    request: StartAppControlSessionRequest,
    manager: tauri::State<'_, AppControlManager>,
) -> Result<AppControlSessionView, AppControlError> {
    commands::start_app_control_session_impl(request, manager)
}

#[tauri::command]
pub fn observe_app_control_session(
    request: AppControlSessionRequest,
    manager: tauri::State<'_, AppControlManager>,
) -> Result<DesktopObservation, AppControlError> {
    commands::observe_app_control_session_impl(request, manager)
}

#[tauri::command]
pub async fn review_and_execute_app_control_action(
    request: ExecuteDesktopActionRequest,
    manager: tauri::State<'_, AppControlManager>,
    approvals: tauri::State<'_, crate::shield_gate::ShieldApprovalManager>,
    app: tauri::AppHandle,
) -> Result<DesktopActionOutcome, AppControlError> {
    commands::review_and_execute_app_control_action_impl(request, manager, approvals, app).await
}

#[cfg(test)]
mod tests;
