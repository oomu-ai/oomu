use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const MAX_OBSERVED_ELEMENTS: usize = 2_000;
pub const MAX_ACTION_TEXT_CHARS: usize = 32_767;
pub const REFERENCE_TTL_MS: i64 = 15_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AppControlState {
    Observing,
    Running,
    Paused,
    Takeover,
    ReturnPending,
    Stopped,
    Completed,
    Failed,
}

impl AppControlState {
    pub(crate) fn active(self) -> bool {
        matches!(
            self,
            Self::Observing | Self::Running | Self::Paused | Self::Takeover | Self::ReturnPending
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopActionKind {
    Focus,
    Press,
    Select,
    TypeText,
    InvokeMenu,
    Scroll,
    DragDrop,
    ChooseFile,
    AppleEvent,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AppControlPauseReason {
    UserInput,
    SecureField,
    AmbiguousTarget,
    RepeatedMismatch,
    UnexpectedNavigation,
    PermissionChanged,
    HiddenWindow,
    ApplicationChanged,
    DriverUnavailable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualifiedAppIcon {
    Finder,
    Preview,
    Mail,
    Calendar,
    Numbers,
    Keynote,
    Excel,
    Powerpoint,
    Generic,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AppControlOutcomeStatus {
    Verified,
    NoChange,
    Failed,
    Paused,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppControlApplicationView {
    pub name: String,
    pub icon: QualifiedAppIcon,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppControlActionView {
    pub kind: DesktopActionKind,
    pub target_label: Option<String>,
    pub will_change_data: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppControlOutcomeView {
    pub status: AppControlOutcomeStatus,
    pub action_kind: DesktopActionKind,
    pub receipt_id: String,
    pub recorded_at_ms: i64,
    pub details_available: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppControlSessionView {
    pub session_id: String,
    pub task_run_id: String,
    pub project_id: String,
    pub state: AppControlState,
    pub application: Option<AppControlApplicationView>,
    pub current_action: Option<AppControlActionView>,
    pub pause_reason: Option<AppControlPauseReason>,
    pub can_pause: bool,
    pub can_take_control: bool,
    pub can_return_to_oomu: bool,
    pub observation_generation: u64,
    pub last_outcome: Option<AppControlOutcomeView>,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetAppControlStatusRequest {
    #[serde(default)]
    pub task_run_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AppControlControl {
    Pause,
    Stop,
    TakeControl,
    ReturnToOomu,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ControlAppControlSessionRequest {
    pub session_id: String,
    pub task_run_id: String,
    pub control: AppControlControl,
}

#[derive(Clone, Debug)]
pub struct StartAppControlSession {
    pub project_id: String,
    pub task_run_id: String,
    pub approved_bundle_ids: Vec<String>,
    pub scoped_file_roots: Vec<PathBuf>,
    pub file_grant_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartAppControlSessionRequest {
    pub project_id: String,
    pub task_run_id: String,
    pub approved_bundle_ids: Vec<String>,
    #[serde(default)]
    pub file_grant_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppControlFileGrantView {
    pub grant_id: String,
    pub file_name: String,
    pub expires_at_ms: i64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppControlSessionRequest {
    pub session_id: String,
    pub task_run_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessibilityPermission {
    Granted,
    Missing,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopApplicationObservation {
    pub bundle_id: String,
    pub display_name: String,
    pub process_id: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopWindowObservation {
    pub window_id: String,
    pub title: String,
    pub visible: bool,
    pub modal: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ElementGeometry {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RedactedScreenshotReceipt {
    pub sha256: String,
    pub width: u32,
    pub height: u32,
    pub redactions_applied: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopObservedElement {
    #[serde(skip)]
    pub(crate) element_key: String,
    pub reference: String,
    pub role: String,
    pub label: Option<String>,
    pub value_digest: Option<String>,
    pub secure: bool,
    pub visible: bool,
    pub enabled: bool,
    pub in_modal: bool,
    pub supported_actions: Vec<DesktopActionKind>,
    pub geometry: Option<ElementGeometry>,
    pub expires_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopObservation {
    pub observation_id: String,
    pub session_id: String,
    pub project_id: String,
    pub task_run_id: String,
    pub revision: u64,
    pub generation: u64,
    pub observed_at_ms: i64,
    pub expires_at_ms: i64,
    pub permission: AccessibilityPermission,
    pub application: DesktopApplicationObservation,
    pub window: DesktopWindowObservation,
    pub focused_element: Option<String>,
    pub elements: Vec<DesktopObservedElement>,
    pub screenshot: Option<RedactedScreenshotReceipt>,
    pub observation_hash: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualifiedMenuCommand {
    Save,
    SaveAs,
    Export,
    NewWindow,
    CloseWindow,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualifiedAppleEvent {
    ActivateApplication,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum DesktopSemanticAction {
    Focus {
        reference: String,
    },
    Press {
        reference: String,
    },
    Select {
        reference: String,
        value: String,
    },
    TypeText {
        reference: String,
        text: String,
    },
    InvokeMenu {
        command: QualifiedMenuCommand,
    },
    Scroll {
        reference: Option<String>,
        amount: i32,
    },
    DragDrop {
        source: String,
        destination: String,
    },
    ChooseFile {
        reference: String,
        file_grant_id: String,
    },
    AppleEvent {
        command: QualifiedAppleEvent,
    },
}

impl DesktopSemanticAction {
    pub fn kind(&self) -> DesktopActionKind {
        match self {
            Self::Focus { .. } => DesktopActionKind::Focus,
            Self::Press { .. } => DesktopActionKind::Press,
            Self::Select { .. } => DesktopActionKind::Select,
            Self::TypeText { .. } => DesktopActionKind::TypeText,
            Self::InvokeMenu { .. } => DesktopActionKind::InvokeMenu,
            Self::Scroll { .. } => DesktopActionKind::Scroll,
            Self::DragDrop { .. } => DesktopActionKind::DragDrop,
            Self::ChooseFile { .. } => DesktopActionKind::ChooseFile,
            Self::AppleEvent { .. } => DesktopActionKind::AppleEvent,
        }
    }

    pub fn references(&self) -> Vec<&str> {
        match self {
            Self::Focus { reference }
            | Self::Press { reference }
            | Self::Select { reference, .. }
            | Self::TypeText { reference, .. }
            | Self::ChooseFile { reference, .. } => vec![reference],
            Self::Scroll { reference, .. } => reference.iter().map(String::as_str).collect(),
            Self::DragDrop {
                source,
                destination,
            } => vec![source, destination],
            Self::InvokeMenu { .. } | Self::AppleEvent { .. } => vec![],
        }
    }

    pub fn will_change_data(&self) -> bool {
        !matches!(
            self,
            Self::Focus { .. }
                | Self::Scroll { .. }
                | Self::AppleEvent {
                    command: QualifiedAppleEvent::ActivateApplication
                }
        )
    }

    pub fn target_label(&self) -> Option<String> {
        None
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectedOutcomeKind {
    NoChange,
    ElementValue,
    ElementState,
    WindowState,
    FileHash,
    ApplicationState,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecuteDesktopActionRequest {
    pub session_id: String,
    pub task_run_id: String,
    pub observation_revision: u64,
    pub action: DesktopSemanticAction,
    pub expected_outcome: ExpectedOutcomeKind,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ObservedPostcondition {
    NoChange,
    ElementValue {
        element_key: String,
        value_sha256: String,
    },
    ElementState {
        element_key: String,
        state: String,
    },
    WindowState {
        window_id: String,
        state: String,
    },
    FileHash {
        canonical_path: PathBuf,
        sha256: String,
    },
    ApplicationState {
        state: String,
    },
}

impl ObservedPostcondition {
    pub fn kind(&self) -> ExpectedOutcomeKind {
        match self {
            Self::NoChange => ExpectedOutcomeKind::NoChange,
            Self::ElementValue { .. } => ExpectedOutcomeKind::ElementValue,
            Self::ElementState { .. } => ExpectedOutcomeKind::ElementState,
            Self::WindowState { .. } => ExpectedOutcomeKind::WindowState,
            Self::FileHash { .. } => ExpectedOutcomeKind::FileHash,
            Self::ApplicationState { .. } => ExpectedOutcomeKind::ApplicationState,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileHashEvidence {
    pub canonical_path: PathBuf,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopOutcomeReceipt {
    pub receipt_id: String,
    pub session_id: String,
    pub project_id: String,
    pub task_run_id: String,
    pub action_kind: DesktopActionKind,
    pub authority_decision_id: String,
    pub before_observation_id: String,
    pub before_observation_hash: String,
    pub after_observation_id: String,
    pub after_observation_hash: String,
    pub action_arguments_hash: String,
    pub driver_receipt_hash: String,
    pub postcondition_hash: String,
    pub file_hashes: Vec<FileHashEvidence>,
    pub status: AppControlOutcomeStatus,
    pub recorded_at_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopActionOutcome {
    pub receipt: DesktopOutcomeReceipt,
    pub observation: DesktopObservation,
    pub session: AppControlSessionView,
}

#[cfg(test)]
mod tests {
    use super::DesktopSemanticAction;

    #[test]
    fn closed_action_schema_rejects_scripts_and_raw_coordinates() {
        assert!(
            serde_json::from_value::<DesktopSemanticAction>(serde_json::json!({
                "kind": "run_script",
                "script": "do shell script"
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<DesktopSemanticAction>(serde_json::json!({
                "kind": "press",
                "reference": "appref_deadbeef",
                "x": 20,
                "y": 30
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<DesktopSemanticAction>(serde_json::json!({
                "kind": "apple_event",
                "command": "save_document"
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<DesktopSemanticAction>(serde_json::json!({
                "kind": "choose_file",
                "reference": "appref_deadbeef",
                "path": "/tmp/guessed"
            }))
            .is_err()
        );
    }
}
