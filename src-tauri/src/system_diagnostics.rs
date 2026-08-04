use crate::agent_manager::AgentManager;
use crate::audit::{PreAlphaAudit, PreAlphaAuditReport, PreAlphaAuditRequest};
use crate::db::{get_database_key, PersistenceEngine};
use crate::foundation::clock::{unix_time_ms_from, unix_time_ms_i64 as unix_time_ms};
use crate::knowledge::KnowledgeStore;
use crate::memory_ledger::{ComparativeAuditRequest, ComparativeAuditResponse, MemoryLedger};
use crate::settings;
use crate::sovereign_identity::SovereignIdentity;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime},
};
use sysinfo::System;

const DEFAULT_MEMORY_AUDIT_QUERY: &str =
    "recurring memory contradictions, stale preferences, and unsafe local configuration drift";
const LOG_TAIL_BYTES: u64 = 64 * 1024;
const LOG_TAIL_LINES: usize = 40;
const MAX_MCP_LOGS: usize = 4;
const ENVIRONMENT_COMMAND_TIMEOUT: Duration = Duration::from_millis(900);
const MAX_ENVIRONMENT_ROWS: usize = 20;
const MAX_PERFORMANCE_MONITOR_ROWS: usize = 24;
const PRIVATE_APP_DATA_REF: &str = "private://app-data";
const PRIVATE_DIAGNOSTICS_REF: &str = "private://diagnostics";
mod native_acceptance;
use native_acceptance::{record_full_disk_access_probe, SystemDiagnosticsRequest};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemDiagnosticsReport {
    pub status: String,
    pub summary: String,
    pub started_at_ms: i64,
    pub completed_at_ms: i64,
    pub duration_ms: u128,
    pub app_data_root: String,
    pub export_root: String,
    pub markdown_report_path: Option<String>,
    pub markdown_exported: bool,
    pub markdown_export_status: String,
    pub system: SystemSnapshot,
    pub database_fragmentation: Vec<DatabaseFragmentationCheck>,
    pub configuration_health: Vec<ConfigurationHealthCheck>,
    pub logs: Vec<LogSnapshot>,
    pub audits: DiagnosticAuditResults,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetacognitivePayload {
    pub active_agent_id: String,
    pub active_model: String,
    pub active_provider: String,
    pub context_limit: usize,
    pub host_os: String,
    pub cpu_architecture: String,
    pub system_memory_used_gb: f64,
    pub system_memory_total_gb: f64,
    pub database_tables_count: usize,
    pub active_mods_bound: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_compilation_stderr: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemSnapshot {
    pub os: String,
    pub arch: String,
    pub host_name: Option<String>,
    pub kernel_version: Option<String>,
    pub os_version: Option<String>,
    pub cpu_count: usize,
    pub total_memory_bytes: u64,
    pub used_memory_bytes: u64,
    pub environment: OperatingEnvironmentSnapshot,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperatingEnvironmentSnapshot {
    pub collected_at_ms: i64,
    pub probe_status: Vec<EnvironmentProbeStatus>,
    pub displays: Vec<DisplaySnapshot>,
    pub ide_windows: Vec<IdeWindowSnapshot>,
    pub node_servers: Vec<NodeServerSnapshot>,
    pub git_workspaces: Vec<GitWorkspaceSnapshot>,
    pub compiler_processes: Vec<CompilerProcessSnapshot>,
    pub performance: AutonomicPerformanceSnapshot,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentProbeStatus {
    pub name: String,
    pub status: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplaySnapshot {
    pub index: usize,
    pub name: String,
    pub frame_x: f64,
    pub frame_y: f64,
    pub frame_width: f64,
    pub frame_height: f64,
    pub is_main: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdeWindowSnapshot {
    pub app_name: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeServerSnapshot {
    pub process_name: String,
    pub pid: u32,
    pub port: u16,
    pub listen_address: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitWorkspaceSnapshot {
    pub path: String,
    pub branch: String,
    pub head_summary: String,
    pub dirty: bool,
    pub changed_files: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompilerProcessSnapshot {
    pub pid: u32,
    pub process_name: String,
    pub command: String,
    pub resident_memory_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutonomicPerformanceSnapshot {
    pub collected_at_ms: i64,
    pub status: String,
    pub memory_warning_threshold_bytes: u64,
    pub probe_status: Vec<EnvironmentProbeStatus>,
    pub monitored_processes: Vec<MonitoredProcessMemorySnapshot>,
    pub warnings: Vec<PerformanceLeakWarning>,
    pub recycle_allowlist: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitoredProcessMemorySnapshot {
    pub pid: u32,
    pub process_name: String,
    pub command: String,
    pub resident_memory_bytes: u64,
    pub category: String,
    pub recycle_allowed: bool,
    pub restart_strategy: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceLeakWarning {
    pub pid: u32,
    pub process_name: String,
    pub category: String,
    pub resident_memory_bytes: u64,
    pub threshold_bytes: u64,
    pub recycle_allowed: bool,
    pub restart_strategy: Option<String>,
    pub detail: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseFragmentationCheck {
    pub name: String,
    pub path: String,
    pub exists: bool,
    pub encrypted: bool,
    pub file_bytes: u64,
    pub wal_bytes: u64,
    pub shm_bytes: u64,
    pub page_count: Option<i64>,
    pub page_size: Option<i64>,
    pub freelist_count: Option<i64>,
    pub free_bytes: Option<i64>,
    pub fragmentation_ratio: Option<f64>,
    pub status: String,
    pub detail: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurationHealthCheck {
    pub name: String,
    pub status: String,
    pub path: Option<String>,
    pub detail: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogSnapshot {
    pub name: String,
    pub path: String,
    pub exists: bool,
    pub size_bytes: u64,
    pub modified_at_ms: Option<i64>,
    pub status: String,
    pub tail_lines: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticAuditResults {
    pub memory_comparative: DiagnosticCommandResult<ComparativeAuditResponse>,
    pub pre_alpha: DiagnosticCommandResult<PreAlphaAuditReport>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum DiagnosticCommandResult<T> {
    Passed { report: T },
    Failed { message: String },
    Skipped { reason: String },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemHardwareProfile {
    pub physical_memory_gb: usize,
    pub physical_memory_available: bool,
    pub processor_tier: String,
    pub cpu_arch: String,
    pub cpu_cores: usize,
    pub cpu_cores_available: bool,
    pub os_name: String,
    pub metal_supported: bool,
    pub metal_probe_available: bool,
    pub max_local_context_budget: usize,
}

#[tauri::command]
pub async fn get_system_diagnostic_context(
    agent_id: Option<String>,
    session_id: Option<String>,
    manager: tauri::State<'_, AgentManager>,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<MetacognitivePayload, String> {
    let manager = manager.inner().clone();
    let persistence = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        collect_metacognitive_payload(agent_id, session_id, &manager, &persistence)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn get_system_hardware_profile() -> Result<SystemHardwareProfile, String> {
    tauri::async_runtime::spawn_blocking(collect_system_hardware_profile)
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn run_system_diagnostics(
    request: SystemDiagnosticsRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    ledger: tauri::State<'_, MemoryLedger>,
    audit: tauri::State<'_, PreAlphaAudit>,
    knowledge: tauri::State<'_, KnowledgeStore>,
    identity: tauri::State<'_, SovereignIdentity>,
) -> Result<SystemDiagnosticsReport, String> {
    record_full_disk_access_probe(&request, persistence.inner()).await?;
    let started_at_ms = unix_time_ms();
    let started = Instant::now();
    let export_root = diagnostics_export_root();

    let database_fragmentation = collect_database_fragmentation(&persistence)?;
    let configuration_health = collect_configuration_health(&export_root);
    let logs = collect_log_snapshots();
    let environment = collect_operating_environment_snapshot().await;
    let system = collect_system_snapshot(environment);

    let memory_comparative = if request.include_memory_audit {
        let channels = if request.memory_channels.is_empty() {
            vec![
                "global".to_string(),
                "agent".to_string(),
                "chat".to_string(),
            ]
        } else {
            request.memory_channels.clone()
        };
        let memory_request = ComparativeAuditRequest {
            query: request
                .memory_query
                .clone()
                .filter(|query| !query.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_MEMORY_AUDIT_QUERY.to_string()),
            channels,
            minimum_recurrence: Some(request.minimum_memory_recurrence.unwrap_or(1).max(1)),
        };
        match crate::memory_ledger::run_memory_comparative_audit(
            memory_request,
            ledger.clone(),
            identity.clone(),
        )
        .await
        {
            Ok(report) => DiagnosticCommandResult::Passed { report },
            Err(error) => DiagnosticCommandResult::Failed {
                message: format!("{}: {}", error.code, error.message),
            },
        }
    } else {
        DiagnosticCommandResult::Skipped {
            reason: "Memory comparative audit was not requested.".to_string(),
        }
    };

    let pre_alpha = if request.include_pre_alpha_audit {
        let pre_alpha_request = PreAlphaAuditRequest {
            runs: Some(request.pre_alpha_runs.unwrap_or(1).clamp(1, 3)),
        };
        match crate::audit::run_pre_alpha_audit(
            pre_alpha_request,
            audit.clone(),
            knowledge.clone(),
            identity.clone(),
            persistence.clone(),
        )
        .await
        {
            Ok(report) => DiagnosticCommandResult::Passed { report },
            Err(error) => DiagnosticCommandResult::Failed {
                message: format!("{}: {}", error.code, error.message),
            },
        }
    } else {
        DiagnosticCommandResult::Skipped {
            reason: "Beta audit was not requested.".to_string(),
        }
    };

    let audits = DiagnosticAuditResults {
        memory_comparative,
        pre_alpha,
    };
    let completed_at_ms = unix_time_ms();
    let status = diagnostics_status(
        &database_fragmentation,
        &configuration_health,
        &audits,
        &system.environment.performance,
    );
    let summary = diagnostics_summary(
        &status,
        &database_fragmentation,
        &configuration_health,
        &audits,
        &system.environment.performance,
    );
    let report_filename = format!("system-diagnostics-{completed_at_ms}.md");
    let mut report = SystemDiagnosticsReport {
        status,
        summary,
        started_at_ms,
        completed_at_ms,
        duration_ms: started.elapsed().as_millis(),
        app_data_root: PRIVATE_APP_DATA_REF.to_string(),
        export_root: PRIVATE_DIAGNOSTICS_REF.to_string(),
        markdown_report_path: Some(format!("{PRIVATE_DIAGNOSTICS_REF}/{report_filename}")),
        markdown_exported: false,
        markdown_export_status: if request.export_markdown {
            "pending".to_string()
        } else {
            "skipped".to_string()
        },
        system,
        database_fragmentation,
        configuration_health,
        logs,
        audits,
    };
    sanitize_diagnostics_for_delivery(&mut report);

    if request.export_markdown {
        let markdown = render_markdown_report(&report);
        write_secure_markdown_report(&export_root, &report_filename, &markdown)?;
        report.markdown_report_path = Some(format!("{PRIVATE_DIAGNOSTICS_REF}/{report_filename}"));
        report.markdown_exported = true;
        report.markdown_export_status = "written".to_string();
    }

    Ok(report)
}

fn sanitize_diagnostics_for_delivery(report: &mut SystemDiagnosticsReport) {
    report.app_data_root = PRIVATE_APP_DATA_REF.to_string();
    report.export_root = PRIVATE_DIAGNOSTICS_REF.to_string();
    report.system.host_name = None;

    for (index, check) in report.database_fragmentation.iter_mut().enumerate() {
        check.path = format!("{PRIVATE_DIAGNOSTICS_REF}/database/{index}");
        check.detail = crate::redaction::redacted_log_text(&check.detail);
    }
    for (index, check) in report.configuration_health.iter_mut().enumerate() {
        check.path = Some(format!("{PRIVATE_DIAGNOSTICS_REF}/configuration/{index}"));
        check.detail = crate::redaction::redacted_log_text(&check.detail);
    }
    for (index, log) in report.logs.iter_mut().enumerate() {
        log.path = format!("{PRIVATE_DIAGNOSTICS_REF}/log/{index}");
        log.tail_lines = redacted_log_tail_metadata(&log.tail_lines);
    }

    sanitize_operating_environment(&mut report.system.environment);
    sanitize_diagnostic_result(&mut report.audits.memory_comparative);
    sanitize_diagnostic_result(&mut report.audits.pre_alpha);

    if let DiagnosticCommandResult::Passed { report: audit } = &mut report.audits.memory_comparative
    {
        audit.query = crate::redaction::redacted_log_text(&audit.query);
        for finding in &mut audit.findings {
            finding.pattern = crate::redaction::redacted_log_text(&finding.pattern);
            finding.source_sessions = finding
                .source_sessions
                .iter()
                .map(|value| crate::redaction::redacted_log_text(value))
                .collect();
        }
    }
    if let DiagnosticCommandResult::Passed { report: audit } = &mut report.audits.pre_alpha {
        audit.report_path = format!("{PRIVATE_DIAGNOSTICS_REF}/pre-alpha/report");
        audit.mission_chronicle_path =
            format!("{PRIVATE_DIAGNOSTICS_REF}/pre-alpha/mission-chronicle");
        audit.release_dir = format!("{PRIVATE_DIAGNOSTICS_REF}/pre-alpha");
        audit.launch_readiness.audit_report_path =
            format!("{PRIVATE_DIAGNOSTICS_REF}/pre-alpha/report");
        audit.runs = audit.runs.iter().map(sanitize_diagnostic_json).collect();
    }
}

fn redacted_log_tail_metadata(lines: &[String]) -> Vec<String> {
    if lines.is_empty() {
        Vec::new()
    } else {
        vec![format!("[redacted-log-tail] lines={}", lines.len())]
    }
}

fn sanitize_diagnostic_result<T>(result: &mut DiagnosticCommandResult<T>) {
    match result {
        DiagnosticCommandResult::Passed { .. } => {}
        DiagnosticCommandResult::Failed { message } => {
            *message = crate::redaction::redacted_log_text(message);
        }
        DiagnosticCommandResult::Skipped { reason } => {
            *reason = crate::redaction::redacted_log_text(reason);
        }
    }
}

fn sanitize_operating_environment(environment: &mut OperatingEnvironmentSnapshot) {
    for status in &mut environment.probe_status {
        status.detail = format!("Probe completed with status {}.", status.status);
    }
    for display in &mut environment.displays {
        display.name = format!("Display {}", display.index);
    }
    for window in &mut environment.ide_windows {
        window.app_name = "IDE".to_string();
        window.title = "[redacted-window-title]".to_string();
    }
    for server in &mut environment.node_servers {
        server.process_name = "node".to_string();
        server.listen_address = format!("local-listener:{}", server.port);
    }
    for (index, workspace) in environment.git_workspaces.iter_mut().enumerate() {
        workspace.path = format!("{PRIVATE_DIAGNOSTICS_REF}/workspace/{index}");
        workspace.branch = "[redacted-branch]".to_string();
        workspace.head_summary = "[redacted-head]".to_string();
    }
    for process in &mut environment.compiler_processes {
        process.process_name = "compiler".to_string();
        process.command = "[redacted-command]".to_string();
    }
    for status in &mut environment.performance.probe_status {
        status.detail = format!("Probe completed with status {}.", status.status);
    }
    for process in &mut environment.performance.monitored_processes {
        process.process_name = "monitored-helper".to_string();
        process.command = "[redacted-command]".to_string();
        process.restart_strategy = process
            .restart_strategy
            .as_ref()
            .map(|_| "[reviewed-native-restart]".to_string());
    }
    for warning in &mut environment.performance.warnings {
        warning.process_name = "monitored-helper".to_string();
        warning.restart_strategy = warning
            .restart_strategy
            .as_ref()
            .map(|_| "[reviewed-native-restart]".to_string());
        warning.detail = "A monitored helper exceeded its reviewed memory threshold.".to_string();
    }
    if !environment.performance.recycle_allowlist.is_empty() {
        environment.performance.recycle_allowlist =
            vec!["[reviewed-native-recycle-allowlist]".to_string()];
    }
}

fn sanitize_diagnostic_json(value: &serde_json::Value) -> serde_json::Value {
    fn scrub(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(object) => {
                for (key, entry) in object {
                    let normalized = key
                        .chars()
                        .filter(|character| character.is_ascii_alphanumeric())
                        .flat_map(char::to_lowercase)
                        .collect::<String>();
                    if normalized != "executionpath"
                        && (normalized == "path"
                            || normalized.ends_with("filepath")
                            || normalized.ends_with("reportpath")
                            || normalized.ends_with("directorypath")
                            || normalized.ends_with("root")
                            || normalized == "releasedir")
                    {
                        *entry = serde_json::Value::String("[redacted-local-path]".to_string());
                    } else {
                        scrub(entry);
                    }
                }
            }
            serde_json::Value::Array(array) => array.iter_mut().for_each(scrub),
            _ => {}
        }
    }

    let mut sanitized = crate::redaction::redact_json_value(value);
    scrub(&mut sanitized);
    sanitized
}

fn collect_database_fragmentation(
    persistence: &PersistenceEngine,
) -> Result<Vec<DatabaseFragmentationCheck>, String> {
    let state_db = PathBuf::from(persistence.db_path());
    let ops_db = state_db
        .parent()
        .map(|parent| parent.join("oomu_ops.db"))
        .unwrap_or_else(|| settings::app_data_root().join("oomu_ops.db"));
    let audit_db = settings::app_data_root()
        .join("release")
        .join("pre_alpha")
        .join("audit_024.sqlite");
    let encrypted_key = get_database_key().ok();

    Ok(vec![
        database_fragmentation_check("State database", &state_db, true, encrypted_key.as_deref()),
        database_fragmentation_check(
            "Ops and memory database",
            &ops_db,
            true,
            encrypted_key.as_deref(),
        ),
        database_fragmentation_check("Beta audit database", &audit_db, false, None),
    ])
}

#[derive(Debug, Clone)]
struct MetacognitiveRuntimeContext {
    agent_id: String,
    model_id: String,
    provider_id: String,
    context_limit: usize,
}

fn collect_metacognitive_payload(
    agent_id: Option<String>,
    session_id: Option<String>,
    manager: &AgentManager,
    persistence: &PersistenceEngine,
) -> Result<MetacognitivePayload, String> {
    let runtime =
        resolve_metacognitive_runtime_context(agent_id, session_id, manager, persistence)?;
    let mut system = System::new_all();
    system.refresh_memory();

    Ok(MetacognitivePayload {
        active_agent_id: runtime.agent_id.clone(),
        active_model: runtime.model_id,
        active_provider: runtime.provider_id,
        context_limit: runtime.context_limit,
        host_os: std::env::consts::OS.to_string(),
        cpu_architecture: std::env::consts::ARCH.to_string(),
        system_memory_used_gb: bytes_to_gib(system.used_memory()),
        system_memory_total_gb: bytes_to_gib(system.total_memory()),
        database_tables_count: count_metacognitive_database_tables(manager, persistence)?,
        active_mods_bound: select_metacognitive_agent_mods(manager, &runtime.agent_id)?,
        last_compilation_stderr: latest_compilation_stderr_from_logs()
            .map(|stderr| compilation_stderr_metadata(&stderr)),
    })
}

fn compilation_stderr_metadata(stderr: &str) -> String {
    format!("[redacted-compilation-stderr] bytes={}", stderr.len())
}

fn resolve_metacognitive_runtime_context(
    agent_id: Option<String>,
    session_id: Option<String>,
    manager: &AgentManager,
    persistence: &PersistenceEngine,
) -> Result<MetacognitiveRuntimeContext, String> {
    let requested_agent_id = clean_optional_diagnostic_id(agent_id);
    let requested_session_id = clean_optional_diagnostic_id(session_id);
    let session_context = if requested_session_id.is_some() || requested_agent_id.is_none() {
        select_metacognitive_session_context(persistence, requested_session_id.as_deref())?
    } else {
        None
    };
    if let Some(requested_session_id) = requested_session_id.as_deref() {
        if session_context.is_none() {
            return Err(format!(
                "Metacognitive runtime context unavailable: chat session '{requested_session_id}' was not found."
            ));
        }
    }

    let session_agent_id = session_context
        .as_ref()
        .map(|context| require_metacognitive_runtime_value("session agent", &context.1))
        .transpose()?;
    if let (Some(requested), Some(session_agent)) =
        (requested_agent_id.as_deref(), session_agent_id.as_deref())
    {
        if requested != session_agent {
            return Err(format!(
                "Metacognitive runtime context mismatch: session agent '{session_agent}' does not match requested agent '{requested}'."
            ));
        }
    }
    let agent_id = requested_agent_id.or(session_agent_id).ok_or_else(|| {
        "Metacognitive runtime context unavailable: no active chat session or agent was found."
            .to_string()
    })?;
    let agent_context =
        select_metacognitive_agent_context(manager, &agent_id)?.ok_or_else(|| {
            format!("Metacognitive runtime context unavailable: agent '{agent_id}' was not found.")
        })?;
    let (provider_id, model_id) = if let Some(context) = session_context.as_ref() {
        (
            require_metacognitive_runtime_value("session provider", &context.2)?,
            require_metacognitive_runtime_value("session model", &context.3)?,
        )
    } else {
        (
            require_metacognitive_runtime_value("agent provider", &agent_context.1)?,
            require_metacognitive_runtime_value("agent model", &agent_context.2)?,
        )
    };
    let active_session_id = session_context.as_ref().map(|context| context.0.as_str());
    let context_limit = match active_session_id {
        Some(id) => select_metacognitive_session_context_limit(persistence, id)
            .map_err(|error| format!("Unable to inspect session context limit: {error}"))?
            .unwrap_or(settings::DEFAULT_CONTEXT_BUDGET),
        None => settings::DEFAULT_CONTEXT_BUDGET,
    };

    Ok(MetacognitiveRuntimeContext {
        agent_id,
        model_id,
        provider_id,
        context_limit,
    })
}

fn require_metacognitive_runtime_value(label: &str, value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!(
            "Metacognitive runtime context unavailable: {label} is missing."
        ));
    }
    Ok(value.to_string())
}

fn clean_optional_diagnostic_id(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn select_metacognitive_session_context(
    persistence: &PersistenceEngine,
    session_id: Option<&str>,
) -> Result<Option<(String, String, String, String)>, String> {
    let connection = persistence
        .open_connection()
        .map_err(|error| error.to_string())?;
    if let Some(session_id) = session_id {
        return connection
            .query_row(
                "
                SELECT id, agent_id, provider_id, model_id
                FROM chat_sessions
                WHERE id = ?1
                ",
                params![session_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .map_err(|error| error.to_string());
    }

    connection
        .query_row(
            "
            SELECT id, agent_id, provider_id, model_id
            FROM chat_sessions
            ORDER BY updated_at_ms DESC
            LIMIT 1
            ",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(|error| error.to_string())
}

fn collect_system_hardware_profile() -> Result<SystemHardwareProfile, String> {
    let telemetry = crate::sys_info::fetch_host_hardware_telemetry();
    let processor_tier = hardware_tier_label(&telemetry);
    let max_local_context_budget =
        crate::sys_info::max_local_context_budget_for_telemetry(&telemetry);

    Ok(SystemHardwareProfile {
        physical_memory_gb: telemetry.physical_ram_gb as usize,
        physical_memory_available: telemetry.physical_ram_available,
        processor_tier,
        cpu_arch: telemetry.cpu_arch,
        cpu_cores: telemetry.cpu_cores,
        cpu_cores_available: telemetry.cpu_cores_available,
        os_name: telemetry.os_name,
        metal_supported: telemetry.metal_supported,
        metal_probe_available: telemetry.metal_probe_available,
        max_local_context_budget,
    })
}

fn hardware_tier_label(telemetry: &crate::sys_info::HostHardwareTelemetry) -> String {
    let budget = crate::sys_info::max_local_context_budget_for_telemetry(telemetry);
    let acceleration = if !telemetry.metal_probe_available {
        "acceleration probe unavailable"
    } else if telemetry.metal_supported {
        "Metal"
    } else {
        "CPU"
    };
    if !telemetry.physical_ram_available {
        return format!(
            "Unknown ({acceleration}, conservative 8K local context; RAM probe unavailable)"
        );
    }
    match budget {
        crate::sys_info::HIGH_SPEC_LOCAL_CONTEXT_BUDGET => {
            format!("High ({acceleration}, 32K local context)")
        }
        crate::sys_info::MID_SPEC_LOCAL_CONTEXT_BUDGET => {
            format!("Mid ({acceleration}, 16K local context)")
        }
        _ => format!("Standard ({acceleration}, 8K local context)"),
    }
}

fn select_metacognitive_agent_context(
    manager: &AgentManager,
    agent_id: &str,
) -> Result<Option<(String, String, String)>, String> {
    let db_path = PathBuf::from(manager.db_path());
    let connection =
        crate::db::open_ops_database_connection(&db_path).map_err(|error| error.to_string())?;
    connection
        .query_row(
            "
            SELECT id, provider_id, model_id
            FROM agent_configs
            WHERE id = ?1
            ",
            params![agent_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| error.to_string())
}

fn select_metacognitive_session_context_limit(
    persistence: &PersistenceEngine,
    session_id: &str,
) -> rusqlite::Result<Option<usize>> {
    Ok(persistence
        .select_session_config(session_id)?
        .map(|config| config.context_budget.max(1) as usize))
}

fn select_metacognitive_agent_mods(
    manager: &AgentManager,
    agent_id: &str,
) -> Result<Vec<String>, String> {
    let db_path = PathBuf::from(manager.db_path());
    let connection = crate::db::open_ops_database_connection(&db_path)
        .map_err(|error| format!("Unable to inspect active agent mods: {error}"))?;
    let mut statement = connection
        .prepare(
            "
            SELECT mod_id
            FROM agent_mods
            WHERE agent_id = ?1
            ORDER BY mod_id COLLATE NOCASE
            ",
        )
        .map_err(|error| format!("Unable to prepare active agent mod inspection: {error}"))?;
    let rows = statement
        .query_map(params![agent_id], |row| row.get::<_, String>(0))
        .map_err(|error| format!("Unable to query active agent mods: {error}"))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| format!("Unable to read active agent mods: {error}"))
}

fn count_metacognitive_database_tables(
    manager: &AgentManager,
    persistence: &PersistenceEngine,
) -> Result<usize, String> {
    let state_connection = persistence
        .open_connection()
        .map_err(|error| format!("Unable to inspect state database tables: {error}"))?;
    let state_count = count_tables(&state_connection)
        .map_err(|error| format!("Unable to count state database tables: {error}"))?;
    let ops_connection = crate::db::open_ops_database_connection(&PathBuf::from(manager.db_path()))
        .map_err(|error| format!("Unable to inspect ops database tables: {error}"))?;
    let ops_count = count_tables(&ops_connection)
        .map_err(|error| format!("Unable to count ops database tables: {error}"))?;
    Ok(state_count + ops_count)
}

fn count_tables(connection: &Connection) -> rusqlite::Result<usize> {
    connection
        .query_row(
            "
        SELECT COUNT(*)
        FROM sqlite_master
        WHERE type = 'table'
          AND name NOT LIKE 'sqlite_%'
        ",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count.max(0) as usize)
}

fn latest_compilation_stderr_from_logs() -> Option<String> {
    let mut logs = collect_log_snapshots()
        .into_iter()
        .filter(|log| {
            log.exists
                && !log.tail_lines.is_empty()
                && (log.name.contains("Next.js") || log.name.contains("Tauri"))
        })
        .collect::<Vec<_>>();
    logs.sort_by(|left, right| right.modified_at_ms.cmp(&left.modified_at_ms));
    logs.into_iter()
        .next()
        .map(|log| truncate_diagnostic_text(&log.tail_lines.join("\n"), 6_000))
        .filter(|value| !value.trim().is_empty())
}

fn bytes_to_gib(bytes: u64) -> f64 {
    let gib = bytes as f64 / 1024.0 / 1024.0 / 1024.0;
    (gib * 100.0).round() / 100.0
}

fn truncate_diagnostic_text(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &value[..end])
}

fn database_fragmentation_check(
    name: &str,
    path: &Path,
    encrypted: bool,
    database_key: Option<&str>,
) -> DatabaseFragmentationCheck {
    let exists = path.exists();
    let file_bytes = file_size(path);
    let wal_bytes = file_size(&sidecar_path(path, "-wal"));
    let shm_bytes = file_size(&sidecar_path(path, "-shm"));
    if !exists {
        return DatabaseFragmentationCheck {
            name: name.to_string(),
            path: path.to_string_lossy().to_string(),
            exists,
            encrypted,
            file_bytes,
            wal_bytes,
            shm_bytes,
            page_count: None,
            page_size: None,
            freelist_count: None,
            free_bytes: None,
            fragmentation_ratio: None,
            status: "missing".to_string(),
            detail: "Database file does not exist yet.".to_string(),
        };
    }

    let connection = match Connection::open(path) {
        Ok(connection) => connection,
        Err(error) => {
            return database_fragmentation_error(
                name,
                path,
                encrypted,
                file_bytes,
                wal_bytes,
                shm_bytes,
                format!("Unable to open database: {error}"),
            );
        }
    };

    if encrypted {
        match database_key {
            Some(key) => {
                if let Err(error) = connection.pragma_update(None, "key", key) {
                    return database_fragmentation_error(
                        name,
                        path,
                        encrypted,
                        file_bytes,
                        wal_bytes,
                        shm_bytes,
                        format!("Unable to unlock encrypted database: {error}"),
                    );
                }
            }
            None => {
                return database_fragmentation_error(
                    name,
                    path,
                    encrypted,
                    file_bytes,
                    wal_bytes,
                    shm_bytes,
                    "Database key is unavailable.".to_string(),
                );
            }
        }
    }

    let _ = connection.pragma_update(None, "query_only", true);
    let page_count = query_pragma_i64(&connection, "page_count");
    let page_size = query_pragma_i64(&connection, "page_size");
    let freelist_count = query_pragma_i64(&connection, "freelist_count");
    let free_bytes = freelist_count
        .zip(page_size)
        .map(|(free, size)| free * size);
    let fragmentation_ratio = page_count
        .zip(freelist_count)
        .and_then(|(pages, free)| (pages > 0).then_some(free as f64 / pages as f64));
    let status = match fragmentation_ratio {
        Some(ratio) if ratio >= 0.20 => "attention",
        Some(_) => "ok",
        None => "unavailable",
    }
    .to_string();
    let detail = match fragmentation_ratio {
        Some(ratio) => format!("{:.1}% of pages are on the freelist.", ratio * 100.0),
        None => "PRAGMA fragmentation metrics were unavailable.".to_string(),
    };

    DatabaseFragmentationCheck {
        name: name.to_string(),
        path: path.to_string_lossy().to_string(),
        exists,
        encrypted,
        file_bytes,
        wal_bytes,
        shm_bytes,
        page_count,
        page_size,
        freelist_count,
        free_bytes,
        fragmentation_ratio,
        status,
        detail,
    }
}

fn database_fragmentation_error(
    name: &str,
    path: &Path,
    encrypted: bool,
    file_bytes: u64,
    wal_bytes: u64,
    shm_bytes: u64,
    detail: String,
) -> DatabaseFragmentationCheck {
    DatabaseFragmentationCheck {
        name: name.to_string(),
        path: path.to_string_lossy().to_string(),
        exists: path.exists(),
        encrypted,
        file_bytes,
        wal_bytes,
        shm_bytes,
        page_count: None,
        page_size: None,
        freelist_count: None,
        free_bytes: None,
        fragmentation_ratio: None,
        status: "unavailable".to_string(),
        detail,
    }
}

fn collect_configuration_health(export_root: &Path) -> Vec<ConfigurationHealthCheck> {
    let app_data_root = settings::app_data_root();
    let settings_path = app_data_root.join("oomu_settings.json");
    let model_dir = settings::resolved_local_model_directory_headless();

    let settings_status = if settings_path.exists() {
        match fs::read_to_string(&settings_path)
            .ok()
            .and_then(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok())
        {
            Some(_) => ("ok", "Settings JSON parses successfully.".to_string()),
            None => (
                "attention",
                "Settings file exists but could not be parsed.".to_string(),
            ),
        }
    } else {
        (
            "ok",
            "Settings file has not been created; defaults are active.".to_string(),
        )
    };

    let model_count = count_model_files(&model_dir);
    vec![
        ConfigurationHealthCheck {
            name: "App data root".to_string(),
            status: if app_data_root.exists() {
                "ok"
            } else {
                "attention"
            }
            .to_string(),
            path: Some(app_data_root.to_string_lossy().to_string()),
            detail: if app_data_root.exists() {
                "App data directory is present.".to_string()
            } else {
                "App data directory is missing.".to_string()
            },
        },
        ConfigurationHealthCheck {
            name: "Settings file".to_string(),
            status: settings_status.0.to_string(),
            path: Some(settings_path.to_string_lossy().to_string()),
            detail: settings_status.1,
        },
        ConfigurationHealthCheck {
            name: "Local model directory".to_string(),
            status: if model_dir.exists() {
                "ok"
            } else {
                "attention"
            }
            .to_string(),
            path: Some(model_dir.to_string_lossy().to_string()),
            detail: if model_dir.exists() {
                format!("{model_count} GGUF model file(s) detected.")
            } else {
                "Configured local model directory does not exist.".to_string()
            },
        },
        ConfigurationHealthCheck {
            name: "Diagnostics export root".to_string(),
            status: if export_root.exists() || export_root.parent().is_some_and(Path::exists) {
                "ok"
            } else {
                "attention"
            }
            .to_string(),
            path: Some(export_root.to_string_lossy().to_string()),
            detail: if export_root.exists() {
                "Diagnostics export directory is present.".to_string()
            } else {
                "Diagnostics export directory will be created under app data.".to_string()
            },
        },
    ]
}

fn collect_log_snapshots() -> Vec<LogSnapshot> {
    let mut paths: Vec<(String, PathBuf)> = Vec::new();
    let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let app_data_logs = settings::app_data_root().join("logs");

    push_log_path(
        &mut paths,
        "Next.js dev log",
        current_dir.join("next-dev.log"),
    );
    push_log_path(&mut paths, "Next.js log", current_dir.join("next.log"));
    push_log_path(
        &mut paths,
        "Tauri stderr log",
        current_dir.join("src-tauri").join("stderr.log"),
    );
    push_log_path(
        &mut paths,
        "Tauri stderr log",
        current_dir.join("stderr.log"),
    );
    push_log_path(
        &mut paths,
        "App Next.js log",
        app_data_logs.join("next.log"),
    );
    push_log_path(&mut paths, "App Tauri log", app_data_logs.join("tauri.log"));

    let mcp_dir = home_dir().join(".oomu").join("logs").join("mcp");
    if let Ok(entries) = fs::read_dir(mcp_dir) {
        let mut log_files = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "log"))
            .collect::<Vec<_>>();
        log_files.sort_by_key(|path| {
            fs::metadata(path)
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(system_time_ms)
                .unwrap_or_default()
        });
        log_files.reverse();
        for path in log_files.into_iter().take(MAX_MCP_LOGS) {
            push_log_path(&mut paths, "MCP stderr log", path);
        }
    }

    paths
        .into_iter()
        .map(|(name, path)| read_log_snapshot(&name, &path))
        .collect()
}

fn push_log_path(paths: &mut Vec<(String, PathBuf)>, name: &str, path: PathBuf) {
    if paths.iter().any(|(_, existing)| existing == &path) {
        return;
    }
    paths.push((name.to_string(), path));
}

fn read_log_snapshot(name: &str, path: &Path) -> LogSnapshot {
    let metadata = fs::metadata(path).ok();
    let exists = metadata.is_some();
    let size_bytes = metadata
        .as_ref()
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let modified_at_ms = metadata
        .as_ref()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(system_time_ms);

    if !exists {
        return LogSnapshot {
            name: name.to_string(),
            path: path.to_string_lossy().to_string(),
            exists,
            size_bytes,
            modified_at_ms,
            status: "missing".to_string(),
            tail_lines: Vec::new(),
        };
    }

    match read_tail(path, size_bytes) {
        Ok(tail_lines) => LogSnapshot {
            name: name.to_string(),
            path: path.to_string_lossy().to_string(),
            exists,
            size_bytes,
            modified_at_ms,
            status: "ok".to_string(),
            tail_lines,
        },
        Err(error) => LogSnapshot {
            name: name.to_string(),
            path: path.to_string_lossy().to_string(),
            exists,
            size_bytes,
            modified_at_ms,
            status: "unavailable".to_string(),
            tail_lines: vec![format!("Unable to read log tail: {error}")],
        },
    }
}

fn collect_system_snapshot(environment: OperatingEnvironmentSnapshot) -> SystemSnapshot {
    let mut system = System::new_all();
    system.refresh_all();
    SystemSnapshot {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        host_name: System::host_name(),
        kernel_version: System::kernel_version(),
        os_version: System::os_version(),
        cpu_count: system.cpus().len(),
        total_memory_bytes: system.total_memory(),
        used_memory_bytes: system.used_memory(),
        environment,
    }
}

pub(crate) async fn collect_operating_environment_snapshot() -> OperatingEnvironmentSnapshot {
    tauri::async_runtime::spawn_blocking(collect_operating_environment_snapshot_sync)
        .await
        .unwrap_or_else(|error| OperatingEnvironmentSnapshot {
            collected_at_ms: unix_time_ms(),
            probe_status: vec![probe_status(
                "operating_environment",
                "failed",
                format!("Environment worker failed: {error}"),
            )],
            displays: Vec::new(),
            ide_windows: Vec::new(),
            node_servers: Vec::new(),
            git_workspaces: Vec::new(),
            compiler_processes: Vec::new(),
            performance: empty_autonomic_performance_snapshot(
                "failed",
                format!("Environment worker failed before performance scan: {error}"),
            ),
        })
}

pub(crate) fn collect_operating_environment_snapshot_sync() -> OperatingEnvironmentSnapshot {
    let displays = thread::spawn(collect_display_snapshots);
    let ide_windows = thread::spawn(collect_ide_window_snapshots);
    let node_servers = thread::spawn(collect_node_server_snapshots);
    let git_workspaces = thread::spawn(collect_git_workspace_snapshots);
    let compiler_processes = thread::spawn(collect_compiler_process_snapshots);
    let performance_processes = thread::spawn(collect_performance_monitor_processes);

    let (displays, display_status) = join_probe(displays, "display_layout");
    let (ide_windows, ide_status) = join_probe(ide_windows, "ide_windows");
    let (node_servers, node_status) = join_probe(node_servers, "node_server_ports");
    let (git_workspaces, git_status) = join_probe(git_workspaces, "git_workspaces");
    let (compiler_processes, compiler_status) =
        join_probe(compiler_processes, "compiler_processes");
    let (performance_processes, performance_status) =
        join_probe(performance_processes, "performance_process_memory");
    let performance =
        build_autonomic_performance_snapshot(performance_processes, performance_status.clone());

    OperatingEnvironmentSnapshot {
        collected_at_ms: unix_time_ms(),
        probe_status: vec![
            display_status,
            ide_status,
            node_status,
            git_status,
            compiler_status,
            performance_status,
        ],
        displays,
        ide_windows,
        node_servers,
        git_workspaces,
        compiler_processes,
        performance,
    }
}

pub(crate) fn format_operating_environment_prompt_context(
    snapshot: &OperatingEnvironmentSnapshot,
) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "Collected at unix_ms: {}",
        snapshot.collected_at_ms
    ));

    if !snapshot.probe_status.is_empty() {
        lines.push("Probe Status".to_string());
        for status in &snapshot.probe_status {
            lines.push(format!("- {}: {}", status.name, status.status));
        }
    }

    lines.push("Display Layout".to_string());
    if snapshot.displays.is_empty() {
        lines.push(environment_empty_or_unavailable(
            snapshot,
            "display_layout",
            "No display layout rows were reported by the macOS probe.",
        ));
    } else {
        for display in &snapshot.displays {
            let main = if display.is_main { " main" } else { "" };
            lines.push(format!(
                "- #{}{} frame {:.0}x{:.0} at ({:.0}, {:.0})",
                display.index,
                main,
                display.frame_width,
                display.frame_height,
                display.frame_x,
                display.frame_y
            ));
        }
    }

    lines.push("Open IDE Windows".to_string());
    if snapshot.ide_windows.is_empty() {
        lines.push(environment_empty_or_unavailable(
            snapshot,
            "ide_windows",
            "No open IDE/editor windows were reported.",
        ));
    } else {
        lines.push(format!(
            "- {} IDE/editor window(s) were reported; titles were withheld.",
            snapshot.ide_windows.len()
        ));
    }

    lines.push("Active Node.js Server Ports".to_string());
    if snapshot.node_servers.is_empty() {
        lines.push(environment_empty_or_unavailable(
            snapshot,
            "node_server_ports",
            "No active Node.js listen ports were detected.",
        ));
    } else {
        for server in &snapshot.node_servers {
            lines.push(format!(
                "- Node.js listener pid {} on port {} (address withheld)",
                server.pid, server.port
            ));
        }
    }

    lines.push("Local Git Workspaces".to_string());
    if snapshot.git_workspaces.is_empty() {
        lines.push(environment_empty_or_unavailable(
            snapshot,
            "git_workspaces",
            "No local Git workspaces were resolved from the active process context.",
        ));
    } else {
        for (index, workspace) in snapshot.git_workspaces.iter().enumerate() {
            let dirty = if workspace.dirty { "dirty" } else { "clean" };
            lines.push(format!(
                "- workspace #{} [{}; {} changed file(s); path, branch, and head withheld]",
                index + 1,
                dirty,
                workspace.changed_files
            ));
        }
    }

    lines.push("Compiler Processes".to_string());
    if snapshot.compiler_processes.is_empty() {
        lines.push(environment_empty_or_unavailable(
            snapshot,
            "compiler_processes",
            "No Turbopack/Next/Vite compiler processes were detected.",
        ));
    } else {
        for process in &snapshot.compiler_processes {
            lines.push(format!(
                "- compiler pid {} rss {} (command withheld)",
                process.pid,
                format_bytes(process.resident_memory_bytes)
            ));
        }
    }

    lines.push("Autonomic Performance Monitor".to_string());
    lines.push(format!(
        "- status {} threshold {}",
        snapshot.performance.status,
        format_bytes(snapshot.performance.memory_warning_threshold_bytes)
    ));
    if snapshot.performance.warnings.is_empty() {
        if snapshot.performance.status == "unavailable" {
            lines.push(
                "- Helper-process threshold status is unavailable because its probe failed."
                    .to_string(),
            );
        } else {
            lines.push(
                "- No allowlisted helper process exceeded the recycling threshold.".to_string(),
            );
        }
    } else {
        for warning in &snapshot.performance.warnings {
            lines.push(format!(
                "- WARNING helper pid {} rss {} category {} recycle_allowed={} (name and restart command withheld)",
                warning.pid,
                format_bytes(warning.resident_memory_bytes),
                warning.category,
                warning.recycle_allowed
            ));
        }
    }
    if snapshot.performance.monitored_processes.is_empty() {
        lines.push(environment_empty_or_unavailable(
            snapshot,
            "performance_process_memory",
            "No monitored display/development helper processes were observed.",
        ));
    } else {
        for process in snapshot.performance.monitored_processes.iter().take(8) {
            lines.push(format!(
                "- helper pid {} rss {} category {} recycle_allowed={} (name, restart, and command withheld)",
                process.pid,
                format_bytes(process.resident_memory_bytes),
                process.category,
                process.recycle_allowed
            ));
        }
    }

    lines.push("Use this as a point-in-time local environment snapshot; verify mutable state with tools before taking filesystem, shell, or compiler actions.".to_string());
    lines.join("\n")
}

fn environment_empty_or_unavailable(
    snapshot: &OperatingEnvironmentSnapshot,
    probe_name: &str,
    observed_empty_message: &str,
) -> String {
    match snapshot
        .probe_status
        .iter()
        .find(|status| status.name == probe_name)
    {
        Some(status) if matches!(status.status.as_str(), "ok" | "empty" | "warning") => {
            format!("- {observed_empty_message}")
        }
        Some(status) => format!(
            "- Unknown: the {probe_name} probe reported {} ({}).",
            status.status, status.detail
        ),
        None => format!("- Unknown: no {probe_name} probe result was recorded."),
    }
}

fn join_probe<T>(
    handle: thread::JoinHandle<(Vec<T>, EnvironmentProbeStatus)>,
    name: &str,
) -> (Vec<T>, EnvironmentProbeStatus) {
    handle.join().unwrap_or_else(|_| {
        (
            Vec::new(),
            probe_status(name, "failed", "Probe worker panicked."),
        )
    })
}

fn collect_display_snapshots() -> (Vec<DisplaySnapshot>, EnvironmentProbeStatus) {
    let args = vec![
        "-l".to_string(),
        "JavaScript".to_string(),
        "-e".to_string(),
        DISPLAY_LAYOUT_JXA.to_string(),
    ];
    match run_command_with_timeout("osascript", &args, ENVIRONMENT_COMMAND_TIMEOUT) {
        Ok(output) if output.timed_out => (
            Vec::new(),
            probe_status(
                "display_layout",
                "timeout",
                format!(
                    "Timed out after {} ms.",
                    ENVIRONMENT_COMMAND_TIMEOUT.as_millis()
                ),
            ),
        ),
        Ok(output) => {
            let displays = parse_display_rows(&output.stdout);
            let status = if !displays.is_empty() {
                probe_status(
                    "display_layout",
                    "ok",
                    format!("Captured {} display layout row(s).", displays.len()),
                )
            } else if output.exit_code == Some(0) {
                probe_status(
                    "display_layout",
                    "empty",
                    "macOS AppKit reported no display rows for this process context.",
                )
            } else {
                probe_status(
                    "display_layout",
                    "unavailable",
                    command_failure_detail(&output),
                )
            };
            (displays, status)
        }
        Err(error) => (
            Vec::new(),
            probe_status(
                "display_layout",
                "unavailable",
                format!("Unable to run osascript display probe: {error}"),
            ),
        ),
    }
}

fn collect_ide_window_snapshots() -> (Vec<IdeWindowSnapshot>, EnvironmentProbeStatus) {
    let args = vec!["-e".to_string(), IDE_WINDOWS_APPLESCRIPT.to_string()];
    match run_command_with_timeout("osascript", &args, ENVIRONMENT_COMMAND_TIMEOUT) {
        Ok(output) if output.timed_out => (
            Vec::new(),
            probe_status(
                "ide_windows",
                "timeout",
                format!(
                    "Timed out after {} ms.",
                    ENVIRONMENT_COMMAND_TIMEOUT.as_millis()
                ),
            ),
        ),
        Ok(output) => {
            let windows = parse_ide_window_rows(&output.stdout);
            let status = if !windows.is_empty() {
                probe_status(
                    "ide_windows",
                    "ok",
                    format!("Captured {} IDE/editor window row(s).", windows.len()),
                )
            } else if output.exit_code == Some(0) {
                probe_status(
                    "ide_windows",
                    "empty",
                    "No target IDE/editor windows were visible to System Events.",
                )
            } else {
                probe_status(
                    "ide_windows",
                    "unavailable",
                    command_failure_detail(&output),
                )
            };
            (windows, status)
        }
        Err(error) => (
            Vec::new(),
            probe_status(
                "ide_windows",
                "unavailable",
                format!("Unable to run AppleScript IDE probe: {error}"),
            ),
        ),
    }
}

fn collect_node_server_snapshots() -> (Vec<NodeServerSnapshot>, EnvironmentProbeStatus) {
    let args = vec![
        "-nP".to_string(),
        "-iTCP".to_string(),
        "-sTCP:LISTEN".to_string(),
    ];
    match run_command_candidates_with_timeout(
        &["/usr/sbin/lsof", "lsof"],
        &args,
        ENVIRONMENT_COMMAND_TIMEOUT,
    ) {
        Ok(output) if output.timed_out => (
            Vec::new(),
            probe_status(
                "node_server_ports",
                "timeout",
                format!(
                    "Timed out after {} ms.",
                    ENVIRONMENT_COMMAND_TIMEOUT.as_millis()
                ),
            ),
        ),
        Ok(output) => {
            let servers = parse_lsof_node_servers(&output.stdout);
            let status = if !servers.is_empty() {
                probe_status(
                    "node_server_ports",
                    "ok",
                    format!("Captured {} Node.js listen port(s).", servers.len()),
                )
            } else if output.exit_code == Some(0)
                || (output.stdout.trim().is_empty() && output.stderr.trim().is_empty())
            {
                probe_status(
                    "node_server_ports",
                    "empty",
                    "No active Node.js server ports were detected.",
                )
            } else {
                probe_status(
                    "node_server_ports",
                    "unavailable",
                    command_failure_detail(&output),
                )
            };
            (servers, status)
        }
        Err(error) => (
            Vec::new(),
            probe_status(
                "node_server_ports",
                "unavailable",
                format!("Unable to run lsof listen-port probe: {error}"),
            ),
        ),
    }
}

fn collect_git_workspace_snapshots() -> (Vec<GitWorkspaceSnapshot>, EnvironmentProbeStatus) {
    let workspaces = candidate_git_workspace_roots()
        .into_iter()
        .filter_map(|root| git_workspace_snapshot(&root))
        .take(MAX_ENVIRONMENT_ROWS)
        .collect::<Vec<_>>();
    let status = if workspaces.is_empty() {
        probe_status(
            "git_workspaces",
            "empty",
            "No Git workspace was found from current or build roots.",
        )
    } else {
        probe_status(
            "git_workspaces",
            "ok",
            format!("Captured {} Git workspace state(s).", workspaces.len()),
        )
    };
    (workspaces, status)
}

fn collect_compiler_process_snapshots() -> (Vec<CompilerProcessSnapshot>, EnvironmentProbeStatus) {
    let args = vec!["-axo".to_string(), "pid=,rss=,comm=,args=".to_string()];
    match run_command_candidates_with_timeout(
        &["/bin/ps", "ps"],
        &args,
        ENVIRONMENT_COMMAND_TIMEOUT,
    ) {
        Ok(output) if output.timed_out => (
            Vec::new(),
            probe_status(
                "compiler_processes",
                "timeout",
                format!(
                    "Timed out after {} ms.",
                    ENVIRONMENT_COMMAND_TIMEOUT.as_millis()
                ),
            ),
        ),
        Ok(output) => {
            let processes = parse_compiler_processes(&output.stdout);
            let status = if !processes.is_empty() {
                probe_status(
                    "compiler_processes",
                    "ok",
                    format!("Captured {} compiler process row(s).", processes.len()),
                )
            } else if output.exit_code == Some(0) {
                probe_status(
                    "compiler_processes",
                    "empty",
                    "No Turbopack/Next/Vite compiler process was detected.",
                )
            } else {
                probe_status(
                    "compiler_processes",
                    "unavailable",
                    command_failure_detail(&output),
                )
            };
            (processes, status)
        }
        Err(error) => (
            Vec::new(),
            probe_status(
                "compiler_processes",
                "unavailable",
                format!("Unable to run process memory probe: {error}"),
            ),
        ),
    }
}

fn collect_performance_monitor_processes(
) -> (Vec<MonitoredProcessMemorySnapshot>, EnvironmentProbeStatus) {
    let mut system = System::new_all();
    system.refresh_all();
    let mut processes = system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            let process_name = process.name().to_string_lossy().to_string();
            let command = process
                .cmd()
                .iter()
                .map(|part| part.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(" ");
            let Some(category) = monitored_performance_category(&process_name, &command) else {
                return None;
            };
            let recycle_policy = crate::native_runtime::autonomic_recycle_policy_for_process(
                &process_name,
                &command,
            );
            Some(MonitoredProcessMemorySnapshot {
                pid: pid.as_u32(),
                process_name: truncate_single_line(&process_name, 100),
                command: truncate_single_line(&command, 240),
                resident_memory_bytes: process.memory(),
                category: category.to_string(),
                recycle_allowed: recycle_policy.is_some_and(|policy| policy.restart_available),
                restart_strategy: recycle_policy
                    .and_then(|policy| policy.restart_strategy.map(str::to_string)),
            })
        })
        .collect::<Vec<_>>();
    processes.sort_by(|left, right| {
        right
            .resident_memory_bytes
            .cmp(&left.resident_memory_bytes)
            .then_with(|| left.process_name.cmp(&right.process_name))
    });
    processes.truncate(MAX_PERFORMANCE_MONITOR_ROWS);

    let status = if processes.is_empty() {
        probe_status(
            "performance_process_memory",
            "empty",
            "No allowlisted display/development helper or WindowServer pressure process was detected.",
        )
    } else {
        let threshold = crate::native_runtime::autonomic_recycle_memory_threshold_bytes();
        let warning_count = performance_leak_warnings_for_processes(&processes, threshold).len();
        let status = if warning_count > 0 { "warning" } else { "ok" };
        probe_status(
            "performance_process_memory",
            status,
            format!(
                "Captured {} monitored helper process row(s); {} above {}.",
                processes.len(),
                warning_count,
                format_bytes(threshold)
            ),
        )
    };
    (processes, status)
}

fn build_autonomic_performance_snapshot(
    monitored_processes: Vec<MonitoredProcessMemorySnapshot>,
    probe_status: EnvironmentProbeStatus,
) -> AutonomicPerformanceSnapshot {
    let threshold = crate::native_runtime::autonomic_recycle_memory_threshold_bytes();
    let warnings = performance_leak_warnings_for_processes(&monitored_processes, threshold);
    let status = if !warnings.is_empty() {
        "warning"
    } else if matches!(
        probe_status.status.as_str(),
        "failed" | "timeout" | "unavailable"
    ) {
        "unavailable"
    } else {
        "ok"
    };
    AutonomicPerformanceSnapshot {
        collected_at_ms: unix_time_ms(),
        status: status.to_string(),
        memory_warning_threshold_bytes: threshold,
        probe_status: vec![probe_status],
        monitored_processes,
        warnings,
        recycle_allowlist: crate::native_runtime::autonomic_recycle_allowlist_labels(),
    }
}

fn empty_autonomic_performance_snapshot(
    status: &str,
    detail: impl Into<String>,
) -> AutonomicPerformanceSnapshot {
    let probe = probe_status("performance_process_memory", status, detail);
    build_autonomic_performance_snapshot(Vec::new(), probe)
}

fn performance_leak_warnings_for_processes(
    processes: &[MonitoredProcessMemorySnapshot],
    threshold: u64,
) -> Vec<PerformanceLeakWarning> {
    processes
        .iter()
        .filter(|process| process.resident_memory_bytes >= threshold && process.recycle_allowed)
        .map(|process| PerformanceLeakWarning {
            pid: process.pid,
            process_name: process.process_name.clone(),
            category: process.category.clone(),
            resident_memory_bytes: process.resident_memory_bytes,
            threshold_bytes: threshold,
            recycle_allowed: process.recycle_allowed,
            restart_strategy: process.restart_strategy.clone(),
            detail: format!(
                "{} pid {} is using {} RSS, above the {} autonomic recycling threshold.",
                process.process_name,
                process.pid,
                format_bytes(process.resident_memory_bytes),
                format_bytes(threshold)
            ),
        })
        .collect()
}

fn parse_display_rows(output: &str) -> Vec<DisplaySnapshot> {
    output
        .lines()
        .filter_map(|line| {
            let parts = line.split('\t').collect::<Vec<_>>();
            if parts.len() < 7 {
                return None;
            }
            Some(DisplaySnapshot {
                index: parts[0].trim().parse().ok()?,
                name: truncate_single_line(parts[1], 80),
                frame_x: parts[2].trim().parse().ok()?,
                frame_y: parts[3].trim().parse().ok()?,
                frame_width: parts[4].trim().parse().ok()?,
                frame_height: parts[5].trim().parse().ok()?,
                is_main: parts[6].trim().eq_ignore_ascii_case("true"),
            })
        })
        .take(MAX_ENVIRONMENT_ROWS)
        .collect()
}

fn parse_ide_window_rows(output: &str) -> Vec<IdeWindowSnapshot> {
    output
        .lines()
        .filter_map(|line| {
            let (app_name, title) = line.split_once('\t')?;
            let app_name = app_name.trim();
            let title = title.trim();
            if app_name.is_empty() || title.is_empty() {
                return None;
            }
            Some(IdeWindowSnapshot {
                app_name: truncate_single_line(app_name, 80),
                title: truncate_single_line(title, 180),
            })
        })
        .take(MAX_ENVIRONMENT_ROWS)
        .collect()
}

fn parse_lsof_node_servers(output: &str) -> Vec<NodeServerSnapshot> {
    output
        .lines()
        .skip(1)
        .filter_map(|line| {
            let parts = line.split_whitespace().collect::<Vec<_>>();
            if parts.len() < 9 {
                return None;
            }
            let process_name = parts[0].trim();
            let haystack = line.to_ascii_lowercase();
            if !is_node_server_process(&haystack) {
                return None;
            }
            let listen_address = parts[8..].join(" ");
            let port = parse_port_from_listen_address(&listen_address)?;
            Some(NodeServerSnapshot {
                process_name: truncate_single_line(process_name, 80),
                pid: parts[1].parse().ok()?,
                port,
                listen_address: truncate_single_line(&listen_address, 120),
            })
        })
        .take(MAX_ENVIRONMENT_ROWS)
        .collect()
}

fn parse_compiler_processes(output: &str) -> Vec<CompilerProcessSnapshot> {
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let pid = parts.next()?.parse().ok()?;
            let rss_kb = parts.next()?.parse::<u64>().ok()?;
            let process_name = parts.next()?.trim();
            let command = parts.collect::<Vec<_>>().join(" ");
            let haystack = format!("{process_name} {command}").to_ascii_lowercase();
            if !is_compiler_process(&haystack) {
                return None;
            }
            Some(CompilerProcessSnapshot {
                pid,
                process_name: truncate_single_line(process_name, 80),
                command: truncate_single_line(&command, 240),
                resident_memory_bytes: rss_kb.saturating_mul(1024),
            })
        })
        .take(MAX_ENVIRONMENT_ROWS)
        .collect()
}

fn parse_port_from_listen_address(address: &str) -> Option<u16> {
    let (_, tail) = address.rsplit_once(':')?;
    let digits = tail
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    digits.parse().ok()
}

fn is_node_server_process(value: &str) -> bool {
    [
        "node", "next", "npm", "pnpm", "yarn", "tsx", "turbo", "vite",
    ]
    .iter()
    .any(|needle| value.contains(needle))
}

fn is_compiler_process(value: &str) -> bool {
    [
        "next-server",
        "next dev",
        "turbopack",
        "turbo",
        "webpack",
        "vite",
    ]
    .iter()
    .any(|needle| value.contains(needle))
}

fn monitored_performance_category(process_name: &str, command: &str) -> Option<&'static str> {
    if let Some(policy) =
        crate::native_runtime::autonomic_recycle_policy_for_process(process_name, command)
    {
        return Some(policy.category);
    }

    if process_name.eq_ignore_ascii_case("WindowServer") {
        return Some("core_macos_pressure_observer");
    }

    None
}

fn candidate_git_workspace_roots() -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut roots = Vec::new();
    if let Ok(current_dir) = std::env::current_dir() {
        push_nearest_git_root(&mut roots, &mut seen, current_dir);
    }
    if let Some(manifest_dir) = crate::runtime_profile::dev_dir() {
        let manifest_dir = PathBuf::from(manifest_dir);
        push_nearest_git_root(&mut roots, &mut seen, manifest_dir.clone());
        if let Some(parent) = manifest_dir.parent() {
            push_nearest_git_root(&mut roots, &mut seen, parent.to_path_buf());
        }
    }
    roots
}

fn push_nearest_git_root(roots: &mut Vec<PathBuf>, seen: &mut BTreeSet<String>, start: PathBuf) {
    let mut candidate = if start.is_file() {
        start
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| start.clone())
    } else {
        start
    };
    loop {
        if candidate.join(".git").exists() {
            let key_path = candidate
                .canonicalize()
                .unwrap_or_else(|_| candidate.clone());
            let key = key_path.to_string_lossy().to_string();
            if seen.insert(key) {
                roots.push(key_path);
            }
            return;
        }
        if !candidate.pop() {
            return;
        }
    }
}

fn git_workspace_snapshot(root: &Path) -> Option<GitWorkspaceSnapshot> {
    let root_text = root.to_string_lossy().to_string();
    let branch_args = vec![
        "-C".to_string(),
        root_text.clone(),
        "branch".to_string(),
        "--show-current".to_string(),
    ];
    let mut branch = run_command_with_timeout("git", &branch_args, ENVIRONMENT_COMMAND_TIMEOUT)
        .ok()
        .and_then(|output| {
            if output.exit_code == Some(0) {
                Some(output.stdout.trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_default();
    if branch.is_empty() {
        let rev_args = vec![
            "-C".to_string(),
            root_text.clone(),
            "rev-parse".to_string(),
            "--short".to_string(),
            "HEAD".to_string(),
        ];
        branch = run_command_with_timeout("git", &rev_args, ENVIRONMENT_COMMAND_TIMEOUT)
            .ok()
            .and_then(|output| {
                if output.exit_code == Some(0) {
                    Some(format!("detached@{}", output.stdout.trim()))
                } else {
                    None
                }
            })?;
    }

    let status_args = vec![
        "-C".to_string(),
        root_text.clone(),
        "status".to_string(),
        "--short".to_string(),
        "--branch".to_string(),
    ];
    let status_output = run_command_with_timeout("git", &status_args, ENVIRONMENT_COMMAND_TIMEOUT)
        .ok()
        .filter(|output| output.exit_code == Some(0));
    let (head_summary, changed_files) = status_output
        .map(|output| {
            let mut lines = output.stdout.lines();
            let head = lines
                .next()
                .unwrap_or("")
                .trim()
                .trim_start_matches("## ")
                .to_string();
            let changed = lines.filter(|line| !line.trim().is_empty()).count();
            (head, changed)
        })
        .unwrap_or_else(|| (branch.clone(), 0));

    Some(GitWorkspaceSnapshot {
        path: root_text,
        branch,
        head_summary,
        dirty: changed_files > 0,
        changed_files,
    })
}

#[derive(Debug)]
struct ProbeCommandOutput {
    stdout: String,
    stderr: String,
    exit_code: Option<i32>,
    timed_out: bool,
}

fn run_command_candidates_with_timeout(
    programs: &[&str],
    args: &[String],
    timeout: Duration,
) -> Result<ProbeCommandOutput, String> {
    let mut last_error = None;
    for program in programs {
        match run_command_with_timeout(program, args, timeout) {
            Ok(output) => return Ok(output),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| "No command candidates were provided.".to_string()))
}

fn run_command_with_timeout(
    program: &str,
    args: &[String],
    timeout: Duration,
) -> Result<ProbeCommandOutput, String> {
    let mut child = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("{program}: {error}"))?;
    let started = Instant::now();
    loop {
        if child
            .try_wait()
            .map_err(|error| format!("{program}: {error}"))?
            .is_some()
        {
            let output = child
                .wait_with_output()
                .map_err(|error| format!("{program}: {error}"))?;
            return Ok(ProbeCommandOutput {
                stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
                exit_code: output.status.code(),
                timed_out: false,
            });
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let output = child
                .wait_with_output()
                .map_err(|error| format!("{program}: {error}"))?;
            return Ok(ProbeCommandOutput {
                stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
                exit_code: output.status.code(),
                timed_out: true,
            });
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn command_failure_detail(output: &ProbeCommandOutput) -> String {
    first_nonempty_line(&output.stderr)
        .or_else(|| first_nonempty_line(&output.stdout))
        .unwrap_or_else(|| {
            output
                .exit_code
                .map(|code| format!("Command exited with status {code}."))
                .unwrap_or_else(|| "Command exited without a status code.".to_string())
        })
}

fn first_nonempty_line(value: &str) -> Option<String> {
    value
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| truncate_single_line(line, 220))
}

fn probe_status(name: &str, status: &str, detail: impl Into<String>) -> EnvironmentProbeStatus {
    EnvironmentProbeStatus {
        name: name.to_string(),
        status: status.to_string(),
        detail: detail.into(),
    }
}

fn truncate_single_line(value: &str, max_chars: usize) -> String {
    let mut normalized = value
        .replace(['\r', '\n'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.chars().count() > max_chars {
        normalized = normalized
            .chars()
            .take(max_chars.saturating_sub(3))
            .collect();
        normalized.push_str("...");
    }
    normalized
}

const DISPLAY_LAYOUT_JXA: &str = r#"ObjC.import('AppKit');
const screens = $.NSScreen.screens;
const mainScreen = $.NSScreen.mainScreen;
const rows = [];
for (let index = 0; index < screens.count; index++) {
  const screen = screens.objectAtIndex(index);
  const frame = screen.frame;
  let name = `Display ${index + 1}`;
  try {
    name = ObjC.unwrap(screen.localizedName);
  } catch (_) {}
  let isMain = false;
  try {
    isMain = screen.isEqual(mainScreen);
  } catch (_) {}
  rows.push([
    index + 1,
    name,
    frame.origin.x,
    frame.origin.y,
    frame.size.width,
    frame.size.height,
    isMain ? 'true' : 'false',
  ].join('\t'));
}
console.log(rows.join('\n'));"#;

const IDE_WINDOWS_APPLESCRIPT: &str = r#"set targetApps to {"Code", "Visual Studio Code", "Cursor", "Xcode", "Terminal", "iTerm2", "Warp"}
set outputRows to ""
tell application "System Events"
  repeat with appName in targetApps
    if exists process (appName as text) then
      tell process (appName as text)
        repeat with nextWindow in windows
          set outputRows to outputRows & (appName as text) & tab & (name of nextWindow as text) & linefeed
        end repeat
      end tell
    end if
  end repeat
end tell
return outputRows"#;

fn diagnostics_status(
    database_fragmentation: &[DatabaseFragmentationCheck],
    configuration_health: &[ConfigurationHealthCheck],
    audits: &DiagnosticAuditResults,
    performance: &AutonomicPerformanceSnapshot,
) -> String {
    let database_attention = database_fragmentation
        .iter()
        .any(|check| matches!(check.status.as_str(), "attention" | "unavailable"));
    let config_attention = configuration_health
        .iter()
        .any(|check| check.status == "attention");
    let audit_attention = audit_has_attention(audits);
    let performance_attention = !performance.warnings.is_empty();
    if database_attention || config_attention || audit_attention || performance_attention {
        "attention_required".to_string()
    } else {
        "passed".to_string()
    }
}

fn diagnostics_summary(
    status: &str,
    database_fragmentation: &[DatabaseFragmentationCheck],
    configuration_health: &[ConfigurationHealthCheck],
    audits: &DiagnosticAuditResults,
    performance: &AutonomicPerformanceSnapshot,
) -> String {
    let database_attention = database_fragmentation
        .iter()
        .filter(|check| matches!(check.status.as_str(), "attention" | "unavailable"))
        .count();
    let config_attention = configuration_health
        .iter()
        .filter(|check| check.status == "attention")
        .count();
    let audit_attention = usize::from(audit_has_attention(audits));
    let performance_attention = performance.warnings.len();
    if status == "passed" {
        "All diagnostics completed without attention flags.".to_string()
    } else {
        format!(
            "{database_attention} database, {config_attention} configuration, {audit_attention} audit, and {performance_attention} performance area(s) need attention."
        )
    }
}

fn audit_has_attention(audits: &DiagnosticAuditResults) -> bool {
    match &audits.memory_comparative {
        DiagnosticCommandResult::Failed { .. } => return true,
        DiagnosticCommandResult::Passed { report } if !report.findings.is_empty() => return true,
        _ => {}
    }
    match &audits.pre_alpha {
        DiagnosticCommandResult::Failed { .. } => true,
        DiagnosticCommandResult::Passed { report } => report.status != "passed",
        DiagnosticCommandResult::Skipped { .. } => false,
    }
}

fn diagnostics_export_root() -> PathBuf {
    settings::app_data_root().join("diagnostics")
}

fn write_secure_markdown_report(
    root: &Path,
    filename: &str,
    content: &str,
) -> Result<PathBuf, String> {
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return Err("Refusing to write diagnostics report with an unsafe filename.".to_string());
    }
    fs::create_dir_all(root)
        .map_err(|_| "Unable to create the private diagnostics export directory.".to_string())?;
    let canonical_root = root
        .canonicalize()
        .map_err(|_| "Unable to resolve the private diagnostics export directory.".to_string())?;
    let report_path = canonical_root.join(filename);
    if !report_path.starts_with(&canonical_root) {
        return Err(
            "Refusing to write diagnostics report outside the export directory.".to_string(),
        );
    }
    fs::write(&report_path, content)
        .map_err(|_| "Unable to write the private diagnostics report.".to_string())?;
    Ok(report_path)
}

fn render_markdown_report(report: &SystemDiagnosticsReport) -> String {
    let mut markdown = String::new();
    markdown.push_str("# OOMU System Diagnostics\n\n");
    markdown.push_str("| Field | Value |\n| --- | --- |\n");
    markdown.push_str(&format!("| Status | {} |\n", md_escape(&report.status)));
    markdown.push_str(&format!("| Summary | {} |\n", md_escape(&report.summary)));
    markdown.push_str(&format!("| Started | {} |\n", report.started_at_ms));
    markdown.push_str(&format!("| Completed | {} |\n", report.completed_at_ms));
    markdown.push_str(&format!("| Duration | {} ms |\n", report.duration_ms));
    markdown.push_str(&format!(
        "| Report Path | {} |\n\n",
        md_escape(
            report
                .markdown_report_path
                .as_deref()
                .unwrap_or("not exported")
        )
    ));

    markdown.push_str("## System Snapshot\n\n");
    markdown.push_str("| Metric | Value |\n| --- | --- |\n");
    markdown.push_str(&format!("| OS | {} |\n", md_escape(&report.system.os)));
    markdown.push_str(&format!("| Arch | {} |\n", md_escape(&report.system.arch)));
    markdown.push_str(&format!(
        "| Host | {} |\n",
        md_escape(report.system.host_name.as_deref().unwrap_or("unknown"))
    ));
    markdown.push_str(&format!("| CPU Count | {} |\n", report.system.cpu_count));
    markdown.push_str(&format!(
        "| Memory | {} used / {} total |\n\n",
        format_bytes(report.system.used_memory_bytes),
        format_bytes(report.system.total_memory_bytes)
    ));

    markdown.push_str("## Operating Environment\n\n");
    markdown.push_str("```text\n");
    markdown.push_str(&format_operating_environment_prompt_context(
        &report.system.environment,
    ));
    markdown.push_str("\n```\n\n");

    markdown.push_str("## Database Fragmentation\n\n");
    markdown.push_str("| Database | Status | File | WAL | Free Pages | Fragmentation | Detail |\n| --- | --- | ---: | ---: | ---: | ---: | --- |\n");
    for check in &report.database_fragmentation {
        markdown.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            md_escape(&check.name),
            md_escape(&check.status),
            format_bytes(check.file_bytes),
            format_bytes(check.wal_bytes),
            check
                .freelist_count
                .map(|value| value.to_string())
                .unwrap_or_else(|| "n/a".to_string()),
            check
                .fragmentation_ratio
                .map(|ratio| format!("{:.1}%", ratio * 100.0))
                .unwrap_or_else(|| "n/a".to_string()),
            md_escape(&check.detail)
        ));
    }
    markdown.push('\n');

    markdown.push_str("## Configuration Health\n\n");
    markdown.push_str("| Check | Status | Detail | Path |\n| --- | --- | --- | --- |\n");
    for check in &report.configuration_health {
        markdown.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            md_escape(&check.name),
            md_escape(&check.status),
            md_escape(&check.detail),
            md_escape(check.path.as_deref().unwrap_or("n/a"))
        ));
    }
    markdown.push('\n');

    markdown.push_str("## Consolidated Audits\n\n");
    markdown.push_str(&audit_markdown(
        "Memory comparative audit",
        &report.audits.memory_comparative,
    ));
    markdown.push_str(&audit_markdown("Beta audit", &report.audits.pre_alpha));

    markdown.push_str("## Log Snapshots\n\n");
    for log in &report.logs {
        markdown.push_str(&format!(
            "### {}\n\n- Status: {}\n- Path: `{}`\n- Size: {}\n\n",
            md_escape(&log.name),
            md_escape(&log.status),
            log.path,
            format_bytes(log.size_bytes)
        ));
        if log.tail_lines.is_empty() {
            markdown.push_str("_No readable log lines captured._\n\n");
        } else {
            markdown.push_str("```text\n");
            for line in &log.tail_lines {
                markdown.push_str(line);
                markdown.push('\n');
            }
            markdown.push_str("```\n\n");
        }
    }

    markdown.push_str("## Logical Certificate\n\n");
    markdown.push_str("### Premises\n\n");
    markdown.push_str("- The diagnostics suite collected local system metrics, database fragmentation, configuration health, bounded log snapshots, and consolidated audit command outcomes.\n");
    markdown.push_str("- The markdown exporter wrote only to a generated filename inside the diagnostics export directory.\n\n");
    markdown.push_str("### Execution Path\n\n");
    markdown.push_str("- Existing state-backed audit commands were invoked through their public Tauri command surfaces.\n");
    markdown.push_str("- SQLite fragmentation metrics were read with PRAGMA checks and reported with transparent unavailable states when needed.\n");
    markdown.push_str("- Log collection was bounded by byte and line limits to avoid exporting excessive local data.\n\n");
    markdown.push_str("### Formal Conclusion\n\n");
    markdown.push_str(&format!("- {}\n", report.summary));

    markdown
}

fn audit_markdown<T: Serialize>(label: &str, result: &DiagnosticCommandResult<T>) -> String {
    match result {
        DiagnosticCommandResult::Passed { report } => {
            let body = serde_json::to_string_pretty(report)
                .unwrap_or_else(|_| "{\"status\":\"serialization_unavailable\"}".to_string());
            format!("### {label}\n\nStatus: passed\n\n```json\n{body}\n```\n\n")
        }
        DiagnosticCommandResult::Failed { message } => {
            format!(
                "### {label}\n\nStatus: failed\n\n{}\n\n",
                md_escape(message)
            )
        }
        DiagnosticCommandResult::Skipped { reason } => {
            format!(
                "### {label}\n\nStatus: skipped\n\n{}\n\n",
                md_escape(reason)
            )
        }
    }
}

fn read_tail(path: &Path, size_bytes: u64) -> std::io::Result<Vec<String>> {
    let mut file = File::open(path)?;
    let start = size_bytes.saturating_sub(LOG_TAIL_BYTES);
    file.seek(SeekFrom::Start(start))?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)?;
    let text = String::from_utf8_lossy(&buffer);
    let mut lines = text.lines().map(str::to_string).collect::<Vec<_>>();
    if start > 0 && !lines.is_empty() {
        lines.remove(0);
    }
    if lines.len() > LOG_TAIL_LINES {
        lines = lines.split_off(lines.len() - LOG_TAIL_LINES);
    }
    Ok(lines)
}

fn query_pragma_i64(connection: &Connection, pragma: &str) -> Option<i64> {
    connection
        .query_row(&format!("PRAGMA {pragma}"), [], |row| row.get::<_, i64>(0))
        .ok()
}

fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut value: OsString = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

fn file_size(path: &Path) -> u64 {
    fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

fn count_model_files(path: &Path) -> usize {
    fs::read_dir(path)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.flatten())
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("gguf"))
        })
        .count()
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn system_time_ms(time: SystemTime) -> Option<i64> {
    unix_time_ms_from(time).map(|millis| millis.min(i64::MAX as u128) as i64)
}

fn default_true() -> bool {
    true
}

fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let bytes = bytes as f64;
    if bytes >= GB {
        format!("{:.2} GB", bytes / GB)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes / MB)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes / KB)
    } else {
        format!("{bytes:.0} B")
    }
}

fn md_escape(value: &str) -> String {
    value.replace('|', "\\|").replace(['\n', '\r'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secure_markdown_writer_rejects_path_traversal() {
        let root =
            std::env::temp_dir().join(format!("oomu-diagnostics-writer-{}", std::process::id()));
        let error = write_secure_markdown_report(&root, "../report.md", "content")
            .expect_err("path traversal should be rejected");
        assert!(error.contains("unsafe filename"));
    }

    #[test]
    fn markdown_escape_removes_table_breaks() {
        assert_eq!(md_escape("alpha|beta\nnext"), "alpha\\|beta next");
    }

    #[test]
    fn metacognitive_runtime_identity_requires_observed_values() {
        assert_eq!(
            require_metacognitive_runtime_value("session model", "  model-observed  ")
                .expect("observed model is accepted"),
            "model-observed"
        );
        let missing = require_metacognitive_runtime_value("session model", "   ")
            .expect_err("missing model identity must not receive a production default");
        assert!(missing.contains("session model is missing"));
    }

    #[test]
    fn environment_parsers_extract_probe_rows() {
        let displays = parse_display_rows("1\tBuilt-in Retina Display\t0\t0\t1728\t1117\ttrue\n");
        assert_eq!(displays.len(), 1);
        assert_eq!(displays[0].name, "Built-in Retina Display");
        assert!(displays[0].is_main);

        let windows = parse_ide_window_rows("Code\tOOMU - sprint_95.rs\nCursor\tMemory Ledger\n");
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].app_name, "Code");

        let lsof = "\
COMMAND   PID USER   FD   TYPE DEVICE SIZE/OFF NODE NAME
node    12345 jeff   22u  IPv4 0x1234      0t0  TCP *:3000 (LISTEN)
Google  44444 jeff   10u  IPv4 0x5678      0t0  TCP *:9222 (LISTEN)
";
        let servers = parse_lsof_node_servers(lsof);
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].port, 3000);

        let processes = parse_compiler_processes(
            "12345 512000 /usr/local/bin/node node next-server (turbo) --port 3000\n",
        );
        assert_eq!(processes.len(), 1);
        assert_eq!(processes[0].resident_memory_bytes, 512000 * 1024);
    }

    #[test]
    fn operating_environment_prompt_context_is_metadata_only() {
        let mut snapshot = OperatingEnvironmentSnapshot {
            collected_at_ms: 42,
            probe_status: vec![probe_status(
                "node_server_ports",
                "ok",
                "Captured 1 Node.js listen port.",
            )],
            displays: vec![DisplaySnapshot {
                index: 1,
                name: "display-client-canary".to_string(),
                frame_x: 0.0,
                frame_y: 0.0,
                frame_width: 2560.0,
                frame_height: 1440.0,
                is_main: true,
            }],
            ide_windows: vec![IdeWindowSnapshot {
                app_name: "Code".to_string(),
                title: "/Volumes/client-canary/secret-project".to_string(),
            }],
            node_servers: vec![NodeServerSnapshot {
                process_name: "node".to_string(),
                pid: 12345,
                port: 3000,
                listen_address: "10.44.55.66:3000 client-address-canary".to_string(),
            }],
            git_workspaces: vec![GitWorkspaceSnapshot {
                path: "/Volumes/client-canary/secret-project".to_string(),
                branch: "client-branch-canary".to_string(),
                head_summary: "client-head-canary".to_string(),
                dirty: true,
                changed_files: 4,
            }],
            compiler_processes: vec![CompilerProcessSnapshot {
                pid: 12345,
                process_name: "node".to_string(),
                command: "next-server --api-key compiler-secret-canary --turbo".to_string(),
                resident_memory_bytes: 512 * 1024 * 1024,
            }],
            performance: empty_autonomic_performance_snapshot("ok", "fixture"),
        };
        let prompt = format_operating_environment_prompt_context(&snapshot);
        assert!(prompt.contains("frame 2560x1440"));
        assert!(prompt.contains("port 3000"));
        assert!(prompt.contains("4 changed file(s)"));
        assert!(prompt.contains("command withheld"));
        assert!(prompt.contains("Autonomic Performance Monitor"));
        for canary in [
            "display-client-canary",
            "client-canary",
            "secret-project",
            "10.44.55.66",
            "client-address-canary",
            "client-branch-canary",
            "client-head-canary",
            "compiler-secret-canary",
            "next-server",
        ] {
            assert!(!prompt.contains(canary), "prompt leaked {canary}: {prompt}");
        }

        sanitize_operating_environment(&mut snapshot);
        let serialized = serde_json::to_string(&snapshot).unwrap();
        for canary in [
            "display-client-canary",
            "client-canary",
            "10.44.55.66",
            "client-branch-canary",
            "client-head-canary",
            "compiler-secret-canary",
            "next-server",
        ] {
            assert!(
                !serialized.contains(canary),
                "response leaked {canary}: {serialized}"
            );
        }
        assert!(serialized.contains("private://diagnostics/workspace/0"));
    }

    #[test]
    fn compilation_stderr_is_reduced_to_metadata() {
        let raw = "/Volumes/client-canary/project failed --api-key raw-secret-canary";
        let metadata = compilation_stderr_metadata(raw);
        assert!(metadata.starts_with("[redacted-compilation-stderr] bytes="));
        assert!(!metadata.contains("client-canary"));
        assert!(!metadata.contains("raw-secret-canary"));
    }

    #[test]
    fn diagnostic_log_tails_are_reduced_to_metadata() {
        let lines = vec![
            "/Volumes/client-canary/private.log message-body-canary".to_string(),
            "Authorization: Bearer raw-secret-canary".to_string(),
        ];
        let metadata = redacted_log_tail_metadata(&lines).join("\n");
        assert_eq!(metadata, "[redacted-log-tail] lines=2");
        for canary in ["client-canary", "message-body-canary", "raw-secret-canary"] {
            assert!(!metadata.contains(canary));
        }
    }

    #[test]
    fn system_hardware_profile_uses_observed_host_telemetry() {
        let profile =
            collect_system_hardware_profile().expect("system hardware profile should resolve");

        assert!(!profile.cpu_arch.trim().is_empty());
        assert!(!profile.os_name.trim().is_empty());
        assert_eq!(profile.cpu_cores_available, profile.cpu_cores > 0);
        assert_eq!(
            profile.max_local_context_budget,
            crate::sys_info::max_local_context_budget_for_physical_memory(
                profile.physical_memory_gb as u64
            )
        );
        assert!(matches!(
            profile.max_local_context_budget,
            8_192 | 16_384 | 32_768
        ));
        assert!(profile.processor_tier.contains("local context"));
    }

    #[test]
    fn performance_monitor_flags_recyclable_displaylink_leaks() {
        let threshold = crate::native_runtime::autonomic_recycle_memory_threshold_bytes();
        let processes = vec![
            MonitoredProcessMemorySnapshot {
                pid: 501,
                process_name: "CrashRestartHelper".to_string(),
                command: "/Library/Application Support/DisplayLink/CrashRestartHelper".to_string(),
                resident_memory_bytes: threshold + 1,
                category: "display_utility".to_string(),
                recycle_allowed: true,
                restart_strategy: Some("open -g -j -a DisplayLink Manager".to_string()),
            },
            MonitoredProcessMemorySnapshot {
                pid: 88,
                process_name: "WindowServer".to_string(),
                command: "/System/Library/PrivateFrameworks/SkyLight.framework/WindowServer"
                    .to_string(),
                resident_memory_bytes: threshold * 2,
                category: "core_macos_pressure_observer".to_string(),
                recycle_allowed: false,
                restart_strategy: None,
            },
        ];

        let warnings = performance_leak_warnings_for_processes(&processes, threshold);

        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].process_name, "CrashRestartHelper");
        assert!(warnings[0].detail.contains("above"));
    }

    #[test]
    fn performance_warning_changes_diagnostics_status() {
        let threshold = crate::native_runtime::autonomic_recycle_memory_threshold_bytes();
        let performance = build_autonomic_performance_snapshot(
            vec![MonitoredProcessMemorySnapshot {
                pid: 501,
                process_name: "CrashRestartHelper".to_string(),
                command: "CrashRestartHelper".to_string(),
                resident_memory_bytes: threshold,
                category: "display_utility".to_string(),
                recycle_allowed: true,
                restart_strategy: Some("open -g -j -a DisplayLink Manager".to_string()),
            }],
            probe_status(
                "performance_process_memory",
                "warning",
                "Captured one leaking helper.",
            ),
        );

        let status = diagnostics_status(&[], &[], &empty_audits(), &performance);
        let summary = diagnostics_summary(&status, &[], &[], &empty_audits(), &performance);

        assert_eq!(status, "attention_required");
        assert!(summary.contains("1 performance"));
    }

    fn empty_audits() -> DiagnosticAuditResults {
        DiagnosticAuditResults {
            memory_comparative: DiagnosticCommandResult::Skipped {
                reason: "fixture".to_string(),
            },
            pre_alpha: DiagnosticCommandResult::Skipped {
                reason: "fixture".to_string(),
            },
        }
    }
}
