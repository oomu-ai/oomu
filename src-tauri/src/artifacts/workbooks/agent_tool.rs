use super::{
    address::{a1, CellAddress},
    commands, validate_workbook, CellValue, CreateWorkbookRequest, ProvenanceReference,
    RecalculationState, SheetVisibility, WorkbookCell, WorkbookDateSystem, WorkbookIr,
    WorkbookPolicy, Worksheet, WorksheetBounds, WORKBOOK_IR_SCHEMA_VERSION,
};
use crate::{
    db::PersistenceEngine,
    foundation::digest::sha256_hex,
    p0_contracts::P0EventEnvelope,
    shield_gate::{CommandStatus, ExecuteCommandResponse},
    sovereign_identity::SovereignIdentity,
    tools::task_tool_runtime::{
        TaskToolExecutionContext, TaskToolFuture, TaskToolRegistration, TaskToolValidation,
    },
};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;

const MAX_TOOL_ARGUMENT_BYTES: usize = 64 * 1024 * 1024;
const MAX_PROJECTED_SOURCE_BYTES: usize = 16 * 1024;
const MAX_PROJECTED_ROWS: usize = 1_000;
const MAX_PROJECTED_COLUMNS: usize = 64;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum CreateSpreadsheetToolRequest {
    Direct(DirectWorkbookRequest),
    Source(SourceProjectionRequest),
    Resolved(ResolvedProjectionRequest),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DirectWorkbookRequest {
    workbook: WorkbookIr,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SourceProjectionRequest {
    source_projection: SpreadsheetSourceProjection,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResolvedProjectionRequest {
    resolved_source_projection: ResolvedSourceProjection,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SpreadsheetSourceProjection {
    from_step: usize,
    collection_pointer: String,
    title: String,
    locale: String,
    sheet_name: String,
    columns: Vec<SpreadsheetProjectionColumn>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SpreadsheetProjectionColumn {
    header: String,
    field: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResolvedSourceProjection {
    source_projection: SpreadsheetSourceProjection,
    workbook: WorkbookIr,
    source_ref: String,
    evidence_ref: String,
    evidence_digest: String,
}

pub(crate) fn register_task_tool() -> Result<(), String> {
    crate::tools::task_tool_runtime::register(TaskToolRegistration {
        operation: "create_spreadsheet",
        validate: validate_registration,
        validate_resolved: validate_resolved_registration,
        resolve: resolve_registration,
        execute: execute_registration,
        planner_context: None,
        schema: crate::tools::spreadsheet_schema::create_spreadsheet_parameters_schema,
        metadata: crate::tools::task_tool_runtime::TaskToolMetadata {
            description: "Create a Project- and Task-bound spreadsheet in the verified private artifact lifecycle.",
            risk_tier: crate::tools::task_tool_runtime::TaskToolRiskTier::FileWrite,
            approval_tier: crate::tools::task_tool_runtime::TaskToolApprovalTier::Background,
            agent_error_code: "workbook_tool_failed",
            agent_error_boundary: "CreateSpreadsheet",
            execution_path: "The native create_spreadsheet tool created a Project-bound private workbook review through the verified artifact lifecycle.",
        },
    })
}

fn validate_registration(arguments: Value) -> Result<TaskToolValidation, String> {
    let request = decode_bounded_request(arguments)?;
    let request = validate_public_request(request)?;
    validation_result(request)
}

fn validate_resolved_registration(arguments: Value) -> Result<TaskToolValidation, String> {
    let request = decode_bounded_request(arguments)?;
    let request = validate_internal_request(request)?;
    validation_result(request)
}

fn decode_bounded_request(arguments: Value) -> Result<CreateSpreadsheetToolRequest, String> {
    if serde_json::to_vec(&arguments)
        .map_err(|error| error.to_string())?
        .len()
        > MAX_TOOL_ARGUMENT_BYTES
    {
        return Err("create_spreadsheet arguments exceed the bounded IR size.".to_string());
    }
    serde_json::from_value::<CreateSpreadsheetToolRequest>(arguments)
        .map_err(|_| "create_spreadsheet arguments do not match the registered schema.".to_string())
}

fn validation_result(request: CreateSpreadsheetToolRequest) -> Result<TaskToolValidation, String> {
    Ok(TaskToolValidation {
        arguments: serde_json::to_value(request).map_err(|error| error.to_string())?,
        potentially_effectful: true,
    })
}

fn validate_public_request(
    request: CreateSpreadsheetToolRequest,
) -> Result<CreateSpreadsheetToolRequest, String> {
    match &request {
        CreateSpreadsheetToolRequest::Direct(request) => validate_direct(request)?,
        CreateSpreadsheetToolRequest::Source(request) => {
            validate_projection(&request.source_projection)?;
        }
        CreateSpreadsheetToolRequest::Resolved(_) => {
            return Err("resolvedSourceProjection is internal runtime state.".to_string())
        }
    }
    Ok(request)
}

fn validate_internal_request(
    request: CreateSpreadsheetToolRequest,
) -> Result<CreateSpreadsheetToolRequest, String> {
    match &request {
        CreateSpreadsheetToolRequest::Direct(request) => validate_direct(request)?,
        CreateSpreadsheetToolRequest::Source(_) => {
            return Err("workbook_source_projection_not_resolved".to_string())
        }
        CreateSpreadsheetToolRequest::Resolved(request) => {
            validate_projection(&request.resolved_source_projection.source_projection)?;
            validate_new_workbook(&request.resolved_source_projection.workbook)?;
            validate_evidence_shape(&request.resolved_source_projection)?;
        }
    }
    Ok(request)
}

fn validate_direct(request: &DirectWorkbookRequest) -> Result<(), String> {
    validate_new_workbook(&request.workbook)?;
    if request
        .workbook
        .worksheets
        .iter()
        .flat_map(|sheet| &sheet.cells)
        .any(|cell| !cell.provenance.is_empty())
    {
        return Err(
            "Direct create_spreadsheet input cannot claim observed-source provenance; use sourceProjection."
                .to_string(),
        );
    }
    Ok(())
}

fn validate_new_workbook(workbook: &WorkbookIr) -> Result<(), String> {
    validate_workbook(workbook)?;
    if workbook.revision != 1 {
        return Err("create_spreadsheet requires workbook revision 1.".to_string());
    }
    Ok(())
}

fn validate_projection(projection: &SpreadsheetSourceProjection) -> Result<(), String> {
    if projection.from_step >= 32
        || !matches!(
            projection.collection_pointer.as_str(),
            "/result" | "/result/value"
        )
        || projection.title.trim().is_empty()
        || projection.title.chars().count() > 240
        || projection.locale.trim().is_empty()
        || !(2..=64).contains(&projection.locale.chars().count())
        || projection.sheet_name.trim().is_empty()
        || projection.sheet_name.chars().count() > 31
        || projection.columns.is_empty()
        || projection.columns.len() > MAX_PROJECTED_COLUMNS
    {
        return Err(
            "create_spreadsheet sourceProjection is outside its bounded contract.".to_string(),
        );
    }
    let mut fields = HashSet::new();
    let mut headers = HashSet::new();
    for column in &projection.columns {
        if column.header.trim().is_empty()
            || column.header.chars().count() > 255
            || !safe_field(&column.field)
            || !fields.insert(column.field.as_str())
            || !headers.insert(column.header.to_ascii_lowercase())
        {
            return Err(
                "create_spreadsheet sourceProjection columns are invalid or duplicated."
                    .to_string(),
            );
        }
    }
    Ok(())
}

fn safe_field(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphabetic() || byte == b'_' || (index > 0 && byte.is_ascii_digit())
        })
}

fn validate_evidence_shape(resolved: &ResolvedSourceProjection) -> Result<(), String> {
    if resolved.source_ref != "connector.tool.completed"
        || resolved.evidence_digest.len() != 64
        || !resolved
            .evidence_digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || !resolved.evidence_ref.starts_with("task-event:taskrun_")
    {
        return Err("create_spreadsheet source evidence receipt is invalid.".to_string());
    }
    Ok(())
}

fn resolve_registration(
    persistence: &PersistenceEngine,
    execution_id: Option<&str>,
    arguments: Value,
    outputs: &[ExecuteCommandResponse],
) -> Result<Value, String> {
    let request =
        serde_json::from_value::<CreateSpreadsheetToolRequest>(arguments).map_err(|_| {
            "create_spreadsheet arguments do not match the registered schema.".to_string()
        })?;
    match request {
        CreateSpreadsheetToolRequest::Source(request) => {
            let output = outputs
                .get(request.source_projection.from_step)
                .ok_or_else(|| "workbook_source_step_not_prior".to_string())?;
            if !output.verified
                || output.operation != "connected_work"
                || !matches!(&output.status, CommandStatus::Completed)
            {
                return Err("workbook_source_step_not_verified".to_string());
            }
            let message = serde_json::from_str::<Value>(&output.message)
                .map_err(|_| "workbook_source_result_invalid".to_string())?;
            if message.get("partial").and_then(Value::as_bool) != Some(false) {
                return Err("workbook_source_result_partial".to_string());
            }
            let resolved = ResolvedSourceProjection {
                source_projection: request.source_projection,
                workbook: empty_workbook_placeholder(),
                source_ref: required_string(&message, "sourceRef")?,
                evidence_ref: required_string(&message, "evidenceRef")?,
                evidence_digest: required_string(&message, "evidenceDigest")?,
            };
            validate_evidence_shape(&resolved)?;
            let authoritative = load_authoritative_result(persistence, execution_id, &resolved)?;
            if message.get("result") != Some(&authoritative) {
                return Err("workbook_source_result_evidence_mismatch".to_string());
            }
            let wrapped = json!({"result": authoritative});
            let workbook = project_rows(
                &resolved.source_projection,
                &wrapped,
                &resolved.source_ref,
                &resolved.evidence_ref,
            )?;
            serde_json::to_value(CreateSpreadsheetToolRequest::Resolved(
                ResolvedProjectionRequest {
                    resolved_source_projection: ResolvedSourceProjection {
                        workbook,
                        ..resolved
                    },
                },
            ))
            .map_err(|error| error.to_string())
        }
        CreateSpreadsheetToolRequest::Direct(request) => {
            serde_json::to_value(CreateSpreadsheetToolRequest::Direct(request))
                .map_err(|error| error.to_string())
        }
        CreateSpreadsheetToolRequest::Resolved(_) => {
            Err("workbook_source_projection_not_from_prior_output".to_string())
        }
    }
}

fn execute_registration<'a>(
    context: TaskToolExecutionContext<'a>,
    arguments: Value,
) -> TaskToolFuture<'a> {
    Box::pin(async move {
        let request =
            serde_json::from_value::<CreateSpreadsheetToolRequest>(arguments).map_err(|_| {
                "create_spreadsheet arguments do not match the registered schema.".to_string()
            })?;
        execute_agent_task_command(
            context.persistence,
            context.identity,
            context.app,
            context.execution_id,
            request,
        )
        .await
    })
}

async fn execute_agent_task_command(
    engine: &PersistenceEngine,
    identity: &SovereignIdentity,
    app: Option<&tauri::AppHandle>,
    execution_id: Option<&str>,
    request: CreateSpreadsheetToolRequest,
) -> Result<ExecuteCommandResponse, String> {
    let execution_id = execution_id
        .ok_or_else(|| "create_spreadsheet requires an active agent Task.".to_string())?;
    let app = app.ok_or_else(|| "create_spreadsheet requires the app runtime.".to_string())?;
    let request = validate_internal_request(request)?;
    let task = crate::tools::task_runtime::require_agent_runtime_task(engine, execution_id)?;
    let project_id = task.project_id.clone();
    let workbook = match request {
        CreateSpreadsheetToolRequest::Direct(request) => request.workbook,
        CreateSpreadsheetToolRequest::Source(_) => {
            return Err("workbook_source_projection_not_resolved".to_string())
        }
        CreateSpreadsheetToolRequest::Resolved(request) => {
            let resolved = request.resolved_source_projection;
            let authoritative = load_authoritative_result(engine, Some(execution_id), &resolved)?;
            let wrapped = json!({"result": authoritative});
            let expected = project_rows(
                &resolved.source_projection,
                &wrapped,
                &resolved.source_ref,
                &resolved.evidence_ref,
            )?;
            if expected != resolved.workbook {
                return Err("workbook_source_projection_tampered".to_string());
            }
            resolved.workbook
        }
    };
    let review = commands::create_workbook_internal(
        CreateWorkbookRequest {
            project_id: project_id.clone(),
            task_id: task.task_id.clone(),
            task_run_id: task.task_run_id.clone(),
            workbook,
        },
        engine,
        identity,
        app,
    )
    .await
    .map_err(|error| error.code)?;
    let message = serde_json::to_string(&json!({
        "artifactId":review.artifact_id,
        "projectId":review.project_id,
        "taskRunId":review.task_run_id,
        "title":review.title,
        "revision":review.current_revision,
        "documentFocus":{"kind":"spreadsheet","id":review.artifact_id},
        "exportReady":review.revisions.first().is_some_and(|revision| revision.exportable),
    }))
    .map_err(|error| error.to_string())?;
    Ok(ExecuteCommandResponse {
        operation: "create_spreadsheet".to_string(),
        status: CommandStatus::Completed,
        message,
        metrics: None,
        claims: vec![format!(
            "CLAIM workbook_artifact_created artifact_id={} task_run_id={} revision={} export_ready={}",
            review.artifact_id,
            review.task_run_id,
            review.current_revision,
            review.revisions.first().is_some_and(|revision| revision.exportable)
        )],
        verified: review
            .revisions
            .first()
            .is_some_and(|revision| revision.technical_evidence_available),
        model_used: None,
    })
}

fn load_authoritative_result(
    engine: &PersistenceEngine,
    execution_id: Option<&str>,
    resolved: &ResolvedSourceProjection,
) -> Result<Value, String> {
    let execution_id = execution_id.ok_or_else(|| "workbook_source_task_required".to_string())?;
    let task = crate::tools::task_runtime::require_agent_runtime_task(engine, execution_id)?;
    let prefix = format!("task-event:{}:", task.task_run_id);
    let sequence = resolved
        .evidence_ref
        .strip_prefix(&prefix)
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| "workbook_source_evidence_ref_invalid".to_string())?;
    let event_json: String = engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .query_row(
            "SELECT event_json FROM task_events WHERE task_run_id=?1 AND sequence=?2",
            params![task.task_run_id, sequence as i64],
            |row| row.get(0),
        )
        .map_err(|_| "workbook_source_evidence_not_found".to_string())?;
    let event = serde_json::from_str::<P0EventEnvelope>(&event_json)
        .map_err(|_| "workbook_source_evidence_invalid".to_string())?;
    let project_id = task.project_id.as_str();
    if event.sequence != sequence
        || event.event_type != resolved.source_ref
        || event.project_id.as_str() != project_id
        || event.task_id.as_str() != task.task_id
        || event.task_run_id.as_ref().map(|run| run.as_str()) != Some(task.task_run_id.as_str())
        || event.payload.get("resultDigest").and_then(Value::as_str)
            != Some(resolved.evidence_digest.as_str())
    {
        return Err("workbook_source_evidence_binding_mismatch".to_string());
    }
    let result = event
        .payload
        .get("resultExcerpt")
        .filter(|value| !value.is_null())
        .cloned()
        .ok_or_else(|| "workbook_source_result_exceeds_projection_bound".to_string())?;
    let encoded =
        serde_json::to_vec(&result).map_err(|_| "workbook_source_result_invalid".to_string())?;
    if encoded.len() > MAX_PROJECTED_SOURCE_BYTES
        || sha256_hex(&encoded) != resolved.evidence_digest
    {
        return Err("workbook_source_result_evidence_mismatch".to_string());
    }
    Ok(result)
}

fn project_rows(
    projection: &SpreadsheetSourceProjection,
    source_result: &Value,
    source_ref: &str,
    evidence_ref: &str,
) -> Result<WorkbookIr, String> {
    validate_projection(projection)?;
    let rows = source_result
        .pointer(&projection.collection_pointer)
        .and_then(Value::as_array)
        .filter(|rows| !rows.is_empty() && rows.len() <= MAX_PROJECTED_ROWS)
        .ok_or_else(|| "workbook_source_rows_invalid".to_string())?;
    let mut cells = Vec::with_capacity((rows.len() + 1) * projection.columns.len());
    for (column_index, column) in projection.columns.iter().enumerate() {
        cells.push(WorkbookCell {
            address: a1(CellAddress {
                row: 1,
                column: column_index as u32 + 1,
            }),
            value: CellValue::Text {
                value: column.header.clone(),
            },
            format_id: None,
            comment: None,
            provenance: vec![],
        });
    }
    for (row_index, row) in rows.iter().enumerate() {
        let row = row
            .as_object()
            .ok_or_else(|| "workbook_source_row_not_object".to_string())?;
        for (column_index, column) in projection.columns.iter().enumerate() {
            let value = row
                .get(&column.field)
                .ok_or_else(|| "workbook_source_field_missing".to_string())?;
            cells.push(WorkbookCell {
                address: a1(CellAddress {
                    row: row_index as u32 + 2,
                    column: column_index as u32 + 1,
                }),
                value: projected_cell_value(value)?,
                format_id: None,
                comment: None,
                provenance: vec![ProvenanceReference {
                    source_ref: source_ref.to_string(),
                    evidence_ref: evidence_ref.to_string(),
                    note: None,
                }],
            });
        }
    }
    let sheet_id = format!(
        "sheet_{}",
        &sha256_hex(format!("{}:{}", projection.title, projection.sheet_name).as_bytes())[..16]
    );
    let workbook = WorkbookIr {
        schema_version: WORKBOOK_IR_SCHEMA_VERSION,
        title: projection.title.clone(),
        locale: projection.locale.clone(),
        date_system: WorkbookDateSystem::Excel1900,
        revision: 1,
        formats: vec![],
        worksheets: vec![Worksheet {
            sheet_id,
            name: projection.sheet_name.clone(),
            bounds: WorksheetBounds {
                row_count: rows.len() as u32 + 1,
                column_count: projection.columns.len() as u32,
            },
            visibility: SheetVisibility::Visible,
            critical: false,
            cells,
            merged_ranges: vec![],
            column_widths: vec![],
            tables: vec![],
            validations: vec![],
            charts: vec![],
        }],
        named_ranges: vec![],
        recalculation: RecalculationState::default(),
        policy: WorkbookPolicy::default(),
    };
    validate_new_workbook(&workbook)?;
    Ok(workbook)
}

fn projected_cell_value(value: &Value) -> Result<CellValue, String> {
    match value {
        Value::Null => Ok(CellValue::Blank),
        Value::Bool(value) => Ok(CellValue::Boolean { value: *value }),
        Value::Number(value) => value
            .as_f64()
            .filter(|value| value.is_finite())
            .map(|value| CellValue::Number { value })
            .ok_or_else(|| "workbook_source_number_invalid".to_string()),
        Value::String(value) if value.chars().count() <= 32_767 && !value.contains('\0') => {
            Ok(CellValue::Text {
                value: value.clone(),
            })
        }
        _ => Err("workbook_source_value_not_scalar".to_string()),
    }
}

fn required_string(value: &Value, field: &str) -> Result<String, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 512)
        .map(str::to_string)
        .ok_or_else(|| "workbook_source_evidence_invalid".to_string())
}

fn empty_workbook_placeholder() -> WorkbookIr {
    WorkbookIr {
        schema_version: WORKBOOK_IR_SCHEMA_VERSION,
        title: "pending".to_string(),
        locale: "en-US".to_string(),
        date_system: WorkbookDateSystem::Excel1900,
        revision: 1,
        formats: vec![],
        worksheets: vec![Worksheet {
            sheet_id: "pending".to_string(),
            name: "Pending".to_string(),
            bounds: WorksheetBounds {
                row_count: 1,
                column_count: 1,
            },
            visibility: SheetVisibility::Visible,
            critical: false,
            cells: vec![WorkbookCell {
                address: "A1".to_string(),
                value: CellValue::Blank,
                format_id: None,
                comment: None,
                provenance: vec![],
            }],
            merged_ranges: vec![],
            column_widths: vec![],
            tables: vec![],
            validations: vec![],
            charts: vec![],
        }],
        named_ranges: vec![],
        recalculation: RecalculationState::default(),
        policy: WorkbookPolicy::default(),
    }
}

#[cfg(test)]
#[path = "agent_tool_tests.rs"]
mod tests;
