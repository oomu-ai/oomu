#[cfg(test)]
mod app_control_planner_tests;
pub(crate) mod create_file_contract;
pub(crate) mod decision_pack_mail;
pub(crate) mod dom_bridge;
pub(crate) use crate::eventkit_calendar;
pub(crate) mod evidence_artifacts;
pub(crate) mod evidence_report_composition;
pub(crate) mod evidence_report_validation;
mod filesystem;
pub(crate) mod milestone_analysis;
pub(crate) mod native_operation_receipt;
pub mod network;
pub(crate) mod official_page;
mod process;
pub(crate) mod project_file;
#[cfg(test)]
mod registered_task_tool_tests;
pub(crate) mod registry;
pub(crate) mod release_recovery;
pub(crate) mod spreadsheet_schema;
pub(crate) mod supplier_exception;
pub(crate) mod system_calendar_event;
pub(crate) mod system_contacts;
pub(crate) mod system_mail;
pub(crate) mod system_mail_send;
pub(crate) mod task_runtime;
mod task_tool_error;
pub(crate) mod task_tool_runtime;
mod telemetry_archive;
pub(crate) mod terminal_contract;
pub mod vision;

pub(crate) fn register_system_task_tools() -> Result<(), String> {
    debug_assert!(native_operation_receipt::contract_is_complete());
    official_page::register_task_tool()?;
    project_file::register_task_tool()?;
    milestone_analysis::register_task_tool()?;
    supplier_exception::register_task_tool()?;
    evidence_report_composition::register_task_tool()?;
    evidence_report_validation::register_task_tool()?;
    system_calendar_event::register_task_tool()?;
    system_mail::register_task_tool()?;
    system_mail_send::register_task_tool()?;
    decision_pack_mail::register_task_tool()?;
    release_recovery::register_task_tools()?;
    evidence_artifacts::register_task_tools()
}

use filesystem::FileSystemTools;
use network::NetworkDiagnosticTools;
use process::ProcessTools;
use telemetry_archive::TelemetryArchiveTools;

use crate::shield_gate::{
    AuthorizedActionBoundary, AuthorizedActions, ExecuteCommandResponse, SystemAuditRequest,
};
use serde::Serialize;
use std::{fs, path::PathBuf};

pub struct ToolRegistry {
    root: PathBuf,
    codebase_root: PathBuf,
}

#[derive(Debug)]
pub struct ToolError {
    pub operation: String,
    pub message: String,
}

#[derive(Debug)]
pub struct ToolOutput {
    pub operation: String,
    pub message: String,
    pub claims: Vec<String>,
}

#[derive(Debug, Serialize)]
struct AuditSnapshot {
    process_count: usize,
    disk_root: String,
    disk_root_is_directory: bool,
    network_status: String,
    internet_reachability_verified: bool,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            root: project_root(),
            codebase_root: crate::shield_gate::development_repo_root(),
        }
    }

    pub fn execute(&self, action: AuthorizedActions) -> Result<ExecuteCommandResponse, ToolError> {
        match action {
            AuthorizedActions::GetSystemMetrics(request) => {
                Ok(crate::shield_gate::get_system_metrics(request))
            }
            AuthorizedActions::FileRead(request) => {
                self.tool_output(FileSystemTools::new(self.root.clone()).read(request)?)
            }
            AuthorizedActions::FileWrite(request) => {
                self.tool_output(FileSystemTools::new(self.root.clone()).write(request)?)
            }
            AuthorizedActions::CodebasePatch(request) => self.tool_output(
                FileSystemTools::new(self.codebase_root.clone()).codebase_patch(request)?,
            ),
            AuthorizedActions::CodebaseCompile(_) => Err(ToolError {
                operation: "codebase_compile".to_string(),
                message: "codebase_compile must be executed through Shield Gate's async runtime."
                    .to_string(),
            }),
            AuthorizedActions::ApprovedExternalFileRead(_)
            | AuthorizedActions::ApprovedExternalFileList(_)
            | AuthorizedActions::ApprovedExternalFileWrite(_)
            | AuthorizedActions::ApprovedFileDelete(_)
            | AuthorizedActions::ApprovedSystemExecution(_) => Err(ToolError {
                operation: "shield_approved_action".to_string(),
                message:
                    "Approved high-risk actions must be executed through Shield Gate directly."
                        .to_string(),
            }),
            AuthorizedActions::FileList(request) => {
                self.tool_output(FileSystemTools::new(self.root.clone()).list(request)?)
            }
            AuthorizedActions::SystemAudit(request) => Ok(self.system_audit(request)),
            AuthorizedActions::TelemetryArchive(request) => self.tool_output(
                TelemetryArchiveTools::new(self.codebase_root.clone()).create(request)?,
            ),
            AuthorizedActions::WebFetch
            | AuthorizedActions::DocumentIndex
            | AuthorizedActions::AskLocalDocumentIndex
            | AuthorizedActions::SovereignDuckDuckGoSearch(_)
            | AuthorizedActions::RegisteredTaskTool(_)
            | AuthorizedActions::AirlockExport(_) => Err(ToolError {
                operation: action.operation_name().to_string(),
                message: "Boundary action requires the agentic loop orchestration layer."
                    .to_string(),
            }),
        }
    }

    fn tool_output(&self, output: ToolOutput) -> Result<ExecuteCommandResponse, ToolError> {
        Ok(ExecuteCommandResponse {
            operation: output.operation,
            status: crate::shield_gate::CommandStatus::Completed,
            message: output.message,
            metrics: None,
            claims: output.claims,
            verified: false,
            model_used: None,
        })
    }

    fn system_audit(&self, request: SystemAuditRequest) -> ExecuteCommandResponse {
        let process_count = match process::observe_process_count() {
            Ok(process_count) => process_count,
            Err(message) => {
                return ExecuteCommandResponse::from_tool_error(ToolError {
                    operation: "system_audit".to_string(),
                    message,
                });
            }
        };
        let root_metadata = match fs::metadata(&self.root) {
            Ok(metadata) if metadata.is_dir() => metadata,
            Ok(_) => {
                return ExecuteCommandResponse::from_tool_error(ToolError {
                    operation: "system_audit".to_string(),
                    message: format!(
                        "System audit root '{}' is not a directory.",
                        self.root.display()
                    ),
                });
            }
            Err(error) => {
                return ExecuteCommandResponse::from_tool_error(ToolError {
                    operation: "system_audit".to_string(),
                    message: format!(
                        "Unable to inspect system audit root '{}': {error}",
                        self.root.display()
                    ),
                });
            }
        };
        debug_assert!(root_metadata.is_dir());
        let network_report = NetworkDiagnosticTools::local_report();
        let snapshot = AuditSnapshot {
            process_count,
            disk_root: self.root.display().to_string(),
            disk_root_is_directory: true,
            network_status: network_report.state.as_str().to_string(),
            internet_reachability_verified: network_report.internet_reachability_verified,
        };
        let process = ProcessTools::diagnostic(&request.scope, process_count);
        let network = NetworkDiagnosticTools::diagnostic(&request.scope);
        let snapshot_json = match serde_json::to_string(&snapshot) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return ExecuteCommandResponse::from_tool_error(ToolError {
                    operation: "system_audit".to_string(),
                    message: format!("Unable to serialize observed system audit data: {error}"),
                });
            }
        };

        ExecuteCommandResponse {
            operation: "system_audit".to_string(),
            status: crate::shield_gate::CommandStatus::Completed,
            message: format!(
                "System audit completed for {}: {snapshot_json}. {} {}",
                request.scope, process.message, network.message
            ),
            metrics: None,
            claims: [
                vec![format!(
                    "CLAIM operation=system_audit observed_process_count={process_count} disk_root_is_directory=true network_state={} internet_reachability_verified={}",
                    network_report.state.as_str(),
                    network_report.internet_reachability_verified
                )],
                process.claims,
                network.claims,
            ]
            .concat(),
            verified: false,
            model_used: None,
        }
    }
}

fn project_root() -> PathBuf {
    crate::settings::app_data_root()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_tool_receipt_does_not_invent_model_identity() {
        let response = ToolRegistry::new()
            .tool_output(ToolOutput {
                operation: "observed_test".to_string(),
                message: "Observed deterministic output.".to_string(),
                claims: vec!["CLAIM observed=true".to_string()],
            })
            .expect("tool output is wrapped");
        assert!(response.model_used.is_none());
    }
}
