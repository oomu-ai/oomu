mod commands;
mod driver;
mod repository;
mod screenshot;
mod transfer;

pub use commands::*;
pub use transfer::BrowserTransferManager;

use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

pub const MAX_SEMANTIC_NODES: usize = 250;
pub const MAX_ACTION_TEXT_CHARS: usize = 16_384;
pub const MAX_SCREENSHOT_BYTES: u64 = 12 * 1024 * 1024;

#[derive(Clone, Default)]
pub struct BrowserAutomationManager {
    state: Arc<Mutex<BrowserAutomationState>>,
}

#[derive(Default)]
struct BrowserAutomationState {
    sessions: HashMap<String, BrowserSession>,
    active_session: Option<String>,
}

#[derive(Clone)]
struct BrowserSession {
    session_id: String,
    task_run_id: String,
    project_id: String,
    canonical_origin: String,
    destination_binding: String,
    native_epoch: u64,
    state: AutomationState,
    document_generation: u64,
    document_marker_key: String,
    element_marker_key: String,
    current_document_marker: Option<String>,
    references: HashMap<String, driver::ElementTarget>,
    current_step: String,
    last_snapshot_at_ms: Option<i64>,
    last_snapshot: Option<BrowserSnapshot>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationState {
    Automating,
    Paused,
    Takeover,
    ReturnPending,
    Stopped,
    Closed,
}

impl AutomationState {
    fn persisted(self) -> &'static str {
        match self {
            Self::Automating => "automating",
            Self::Paused | Self::ReturnPending => "paused",
            Self::Takeover => "takeover",
            Self::Stopped => "stopped",
            Self::Closed => "closed",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserSessionView {
    pub session_id: String,
    pub task_run_id: String,
    pub project_id: String,
    pub canonical_origin: String,
    pub destination_binding: String,
    pub state: AutomationState,
    pub document_generation: u64,
    pub current_step: String,
    pub last_snapshot_at_ms: Option<i64>,
    pub snapshot: Option<BrowserSnapshot>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserSnapshot {
    pub document_generation: u64,
    pub url: String,
    pub title: String,
    pub captured_at_ms: i64,
    pub nodes: Vec<SemanticNode>,
    pub possible_prompt_injection: bool,
    pub protected_interruption: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticNode {
    pub role: String,
    pub name: String,
    pub value_class: String,
    pub visible: bool,
    pub enabled: bool,
    pub reference: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartBrowserAutomationRequest {
    pub task_run_id: String,
    pub project_id: String,
    #[serde(default)]
    pub project_policy_consent: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserSessionRequest {
    pub session_id: String,
    pub task_run_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserControlRequest {
    pub session_id: String,
    pub task_run_id: String,
    pub control: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum BrowserAction {
    Status,
    Navigate {
        url: String,
    },
    Snapshot,
    Screenshot,
    Click {
        reference: String,
    },
    Type {
        reference: String,
        text: String,
    },
    Select {
        reference: String,
        value: String,
    },
    PressKey {
        key: String,
    },
    Scroll {
        delta_y: i32,
    },
    UploadApprovedFile {
        reference: String,
        upload_grant_id: String,
    },
    DownloadToQuarantine {
        reference: String,
    },
    Wait {
        milliseconds: u64,
    },
    Close,
}

impl BrowserAction {
    fn kind(&self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Navigate { .. } => "navigate",
            Self::Snapshot => "snapshot",
            Self::Screenshot => "screenshot",
            Self::Click { .. } => "click",
            Self::Type { .. } => "type",
            Self::Select { .. } => "select",
            Self::PressKey { .. } => "press_key",
            Self::Scroll { .. } => "scroll",
            Self::UploadApprovedFile { .. } => "upload_approved_file",
            Self::DownloadToQuarantine { .. } => "download_to_quarantine",
            Self::Wait { .. } => "wait",
            Self::Close => "close",
        }
    }

    fn reference(&self) -> Option<&str> {
        match self {
            Self::Click { reference }
            | Self::Type { reference, .. }
            | Self::Select { reference, .. }
            | Self::UploadApprovedFile { reference, .. }
            | Self::DownloadToQuarantine { reference } => Some(reference),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecuteBrowserActionRequest {
    pub session_id: String,
    pub task_run_id: String,
    pub project_id: String,
    pub action: BrowserAction,
    pub step: String,
    pub expected_postcondition: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserActionResult {
    pub action_id: String,
    pub action_kind: String,
    pub state: String,
    pub observation: Option<BrowserSnapshot>,
    pub screenshot_path: Option<String>,
    pub downloads: Vec<BrowserDownloadView>,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserDownloadView {
    pub download_id: String,
    pub file_name: String,
    pub mime_type: String,
    pub byte_count: u64,
    pub sha256: String,
    pub state: String,
}

fn opaque_id(prefix: &str) -> String {
    let mut bytes = [0_u8; 18];
    OsRng.fill_bytes(&mut bytes);
    format!("{prefix}_{}", hex::encode(bytes))
}

impl BrowserSession {
    fn view(&self) -> BrowserSessionView {
        BrowserSessionView {
            session_id: self.session_id.clone(),
            task_run_id: self.task_run_id.clone(),
            project_id: self.project_id.clone(),
            canonical_origin: self.canonical_origin.clone(),
            destination_binding: self.destination_binding.clone(),
            state: self.state,
            document_generation: self.document_generation,
            current_step: self.current_step.clone(),
            last_snapshot_at_ms: self.last_snapshot_at_ms,
            snapshot: self.last_snapshot.clone(),
        }
    }
}

impl BrowserAutomationManager {
    pub(crate) fn read_snapshot(
        &self,
        session_id: &str,
        task_run_id: &str,
    ) -> Result<BrowserSnapshot, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "Browser automation state is unavailable.".to_string())?;
        let session = state
            .sessions
            .get(session_id)
            .filter(|session| session.task_run_id == task_run_id)
            .ok_or_else(|| "Delegated browser session was not found for this Task.".to_string())?;
        session.last_snapshot.clone().ok_or_else(|| {
            "Delegated browser source requires an observed semantic snapshot.".to_string()
        })
    }
}
