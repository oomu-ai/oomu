use super::{
    system_calendar_event, system_mail,
    task_runtime::{record_event, require_agent_runtime_task},
    task_tool_runtime::{
        TaskToolApprovalTier, TaskToolExecutionContext, TaskToolFuture, TaskToolMetadata,
        TaskToolRegistration, TaskToolRiskTier, TaskToolValidation,
    },
};
use crate::{
    p0_contracts::EvidenceClass,
    shield_gate::{CommandStatus, ExecuteCommandResponse},
};
use chrono::{DateTime, Local, NaiveDate};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
};

pub(crate) const PREPARE_OPERATION: &str = "prepare_release_recovery_agenda";
pub(crate) const CALENDAR_OPERATION: &str = "create_release_recovery_calendar_event";
pub(crate) const MAIL_OPERATION: &str = "draft_release_recovery_email";
const MAX_FIXTURE_BYTES: usize = 1_048_576;
const MAX_RECEIPT_BYTES: usize = 128 * 1024;
const EXPECTED_AGENDA_ITEMS: usize = 5;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PrepareAgendaRequest {
    input_path: String,
    output_path: String,
    day: String,
    window_start_local: String,
    window_end_local: String,
    duration_minutes: i64,
    agenda_item_count: usize,
    locale: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MilestoneInput {
    milestone_id: String,
    name: String,
    target_date: String,
    status: String,
    owner: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MilestoneFact {
    milestone_id: String,
    name: String,
    target_date: String,
    status: String,
    owner: String,
    completed: bool,
    overdue: bool,
    unfinished: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AgendaReceipt {
    status: String,
    verified: bool,
    input_path: String,
    input_sha256: String,
    output_path: String,
    output_sha256: String,
    byte_length: u64,
    as_of_date: String,
    start_date: String,
    end_date: String,
    time_zone: String,
    proposed_time: String,
    agenda_items: Vec<String>,
    milestone_facts: Vec<MilestoneFact>,
    event_notes: String,
    mail_body: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CalendarPlanRequest {
    calendar_name: String,
    title: String,
    agenda_step: usize,
    availability: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CalendarResolvedRequest {
    calendar_name: String,
    title: String,
    start_date: String,
    end_date: String,
    location: String,
    notes: String,
    availability: String,
    agenda_step: usize,
    agenda_sha256: String,
    output_path: String,
    output_sha256: String,
    byte_length: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MailPlanRequest {
    to: String,
    subject: String,
    agenda_step: usize,
    calendar_step: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MailResolvedRequest {
    to: String,
    subject: String,
    body: String,
    start_date: String,
    end_date: String,
    agenda_items: Vec<String>,
    agenda_step: usize,
    calendar_step: usize,
    agenda_sha256: String,
    output_path: String,
    output_sha256: String,
    byte_length: u64,
}

pub(crate) fn register_task_tools() -> Result<(), String> {
    crate::tools::task_tool_runtime::register(TaskToolRegistration {
        operation: PREPARE_OPERATION,
        validate: validate_prepare,
        validate_resolved: validate_prepare,
        resolve: crate::tools::task_tool_runtime::identity_resolver,
        execute: execute_prepare,
        planner_context: None,
        schema: prepare_schema,
        metadata: TaskToolMetadata {
            description: "Read one approved milestone fixture, inspect Calendar without changing it, freeze one exact conflict-free slot, and create one verified recovery agenda file.",
            risk_tier: TaskToolRiskTier::FileWrite,
            approval_tier: TaskToolApprovalTier::Visual,
            agent_error_code: "release_recovery_agenda_failed",
            agent_error_boundary: "ReleaseRecoveryAgenda",
            execution_path: "The native recovery-agenda tool read the exact fixture, inspected the real Calendar window, froze one conflict-free slot, created one exact Markdown file, and reopened it to verify its bytes and digest.",
        },
    })?;
    crate::tools::task_tool_runtime::register(TaskToolRegistration {
        operation: CALENDAR_OPERATION,
        validate: validate_calendar_plan,
        validate_resolved: validate_calendar_resolved,
        resolve: resolve_calendar,
        execute: execute_calendar,
        planner_context: None,
        schema: calendar_schema,
        metadata: TaskToolMetadata {
            description: "Create one tentative macOS Calendar event from the frozen, verified recovery-agenda receipt.",
            risk_tier: TaskToolRiskTier::SystemExec,
            approval_tier: TaskToolApprovalTier::Explicit,
            agent_error_code: "calendar_event_creation_failed",
            agent_error_boundary: "CreateReleaseRecoveryCalendarEvent",
            execution_path: "The native recovery Calendar tool resolved the exact frozen slot and agenda from the verified file receipt, rechecked conflicts, created one approved tentative event, and verified EventKit readback.",
        },
    })?;
    crate::tools::task_tool_runtime::register(TaskToolRegistration {
        operation: MAIL_OPERATION,
        validate: validate_mail_plan,
        validate_resolved: validate_mail_resolved,
        resolve: resolve_mail,
        execute: execute_mail,
        planner_context: None,
        schema: mail_schema,
        metadata: TaskToolMetadata {
            description: "Create and verify one visible unsent macOS Mail draft bound to the verified recovery agenda and Calendar receipt. This tool cannot send.",
            risk_tier: TaskToolRiskTier::FileWrite,
            approval_tier: TaskToolApprovalTier::Explicit,
            agent_error_code: "mail_draft_creation_failed",
            agent_error_boundary: "DraftReleaseRecoveryEmail",
            execution_path: "The native recovery Mail tool resolved the same frozen time and five agenda items from verified receipts, saved one unsent draft, and verified exact recipients, subject, body, uniqueness, and zero sent copies.",
        },
    })
}

fn prepare_schema() -> Value {
    json!({
        "type":"object",
        "properties":{
            "inputPath":{"type":"string","minLength":1,"maxLength":4096},
            "outputPath":{"type":"string","minLength":1,"maxLength":4096},
            "day":{"type":"string","enum":["next_weekday"]},
            "windowStartLocal":{"type":"string","enum":["13:00"]},
            "windowEndLocal":{"type":"string","enum":["16:00"]},
            "durationMinutes":{"type":"integer","enum":[30]},
            "agendaItemCount":{"type":"integer","enum":[5]},
            "locale":{"type":"string","enum":["en-US"]}
        },
        "required":["inputPath","outputPath","day","windowStartLocal","windowEndLocal","durationMinutes","agendaItemCount","locale"],
        "additionalProperties":false
    })
}

fn calendar_schema() -> Value {
    json!({
        "type":"object",
        "properties":{
            "calendarName":{"type":"string","minLength":1,"maxLength":512},
            "title":{"type":"string","minLength":1,"maxLength":2048},
            "agendaStep":{"type":"integer","minimum":0,"maximum":31},
            "availability":{"type":"string","enum":["tentative"]}
        },
        "required":["calendarName","title","agendaStep","availability"],
        "additionalProperties":false
    })
}

fn mail_schema() -> Value {
    json!({
        "type":"object",
        "properties":{
            "to":{"type":"string","minLength":3,"maxLength":4096},
            "subject":{"type":"string","minLength":1,"maxLength":998},
            "agendaStep":{"type":"integer","minimum":0,"maximum":31},
            "calendarStep":{"type":"integer","minimum":0,"maximum":31}
        },
        "required":["to","subject","agendaStep","calendarStep"],
        "additionalProperties":false
    })
}

fn validate_prepare(arguments: Value) -> Result<TaskToolValidation, String> {
    let mut request = serde_json::from_value::<PrepareAgendaRequest>(arguments).map_err(|_| {
        "prepare_release_recovery_agenda arguments do not match the registered schema.".to_string()
    })?;
    request.input_path = request.input_path.trim().to_string();
    request.output_path = request.output_path.trim().to_string();
    request.locale = request.locale.trim().to_string();
    if request.day != "next_weekday"
        || request.window_start_local != "13:00"
        || request.window_end_local != "16:00"
        || request.duration_minutes != 30
        || request.agenda_item_count != EXPECTED_AGENDA_ITEMS
        || request.locale != "en-US"
        || !bounded_absolute_file(&request.input_path, "json")
        || !bounded_absolute_file(&request.output_path, "md")
        || request.input_path == request.output_path
    {
        return Err(
            "prepare_release_recovery_agenda request is outside the bounded contract.".to_string(),
        );
    }
    Ok(TaskToolValidation {
        arguments: serde_json::to_value(request).map_err(|error| error.to_string())?,
        potentially_effectful: true,
    })
}

fn validate_calendar_plan(arguments: Value) -> Result<TaskToolValidation, String> {
    let mut request = serde_json::from_value::<CalendarPlanRequest>(arguments).map_err(|_| {
        "create_release_recovery_calendar_event arguments do not match the planning schema."
            .to_string()
    })?;
    request.calendar_name = request.calendar_name.trim().to_string();
    request.title = request.title.trim().to_string();
    if request.calendar_name.is_empty()
        || request.calendar_name.chars().count() > 512
        || request.title.is_empty()
        || request.title.chars().count() > 2_048
        || request.agenda_step > 31
        || request.availability != "tentative"
        || contains_nul(&request.calendar_name)
        || contains_nul(&request.title)
    {
        return Err(
            "create_release_recovery_calendar_event request is outside the bounded contract."
                .to_string(),
        );
    }
    Ok(TaskToolValidation {
        arguments: serde_json::to_value(request).map_err(|error| error.to_string())?,
        potentially_effectful: true,
    })
}

fn validate_calendar_resolved(arguments: Value) -> Result<TaskToolValidation, String> {
    let request = serde_json::from_value::<CalendarResolvedRequest>(arguments).map_err(|_| {
        "create_release_recovery_calendar_event resolved arguments are invalid.".to_string()
    })?;
    if request.agenda_step > 31
        || !resolved_agenda_binding_is_valid(
            &request.output_path,
            &request.output_sha256,
            request.byte_length,
            &request.agenda_sha256,
        )
    {
        return Err(
            "create_release_recovery_calendar_event resolved arguments are invalid.".to_string(),
        );
    }
    let base = calendar_base_arguments(&request);
    system_calendar_event::validate_registration(base)?;
    Ok(TaskToolValidation {
        arguments: serde_json::to_value(request).map_err(|error| error.to_string())?,
        potentially_effectful: true,
    })
}

fn validate_mail_plan(arguments: Value) -> Result<TaskToolValidation, String> {
    let mut request = serde_json::from_value::<MailPlanRequest>(arguments).map_err(|_| {
        "draft_release_recovery_email arguments do not match the planning schema.".to_string()
    })?;
    request.to = request.to.trim().to_string();
    request.subject = request.subject.trim().to_string();
    if !one_email_address(&request.to)
        || request.subject.is_empty()
        || request.subject.chars().count() > 998
        || request.agenda_step > 31
        || request.calendar_step > 31
        || request.agenda_step >= request.calendar_step
        || contains_nul(&request.subject)
    {
        return Err(
            "draft_release_recovery_email request is outside the bounded contract.".to_string(),
        );
    }
    Ok(TaskToolValidation {
        arguments: serde_json::to_value(request).map_err(|error| error.to_string())?,
        potentially_effectful: true,
    })
}

fn validate_mail_resolved(arguments: Value) -> Result<TaskToolValidation, String> {
    let request = serde_json::from_value::<MailResolvedRequest>(arguments)
        .map_err(|_| "draft_release_recovery_email resolved arguments are invalid.".to_string())?;
    if !resolved_agenda_binding_is_valid(
        &request.output_path,
        &request.output_sha256,
        request.byte_length,
        &request.agenda_sha256,
    ) || request.agenda_items.len() != EXPECTED_AGENDA_ITEMS
        || request
            .agenda_items
            .iter()
            .any(|item| item.trim().is_empty())
    {
        return Err("draft_release_recovery_email resolved arguments are invalid.".to_string());
    }
    system_mail::validate_registration(mail_base_arguments(&request))?;
    Ok(TaskToolValidation {
        arguments: serde_json::to_value(request).map_err(|error| error.to_string())?,
        potentially_effectful: true,
    })
}

fn bounded_absolute_file(path: &str, extension: &str) -> bool {
    let path = Path::new(path);
    path.is_absolute()
        && path.as_os_str().len() <= 4096
        && !path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        && path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case(extension))
        && !contains_nul(&path.to_string_lossy())
}

fn resolved_agenda_binding_is_valid(
    output_path: &str,
    output_sha256: &str,
    byte_length: u64,
    agenda_sha256: &str,
) -> bool {
    bounded_absolute_file(output_path, "md")
        && byte_length > 0
        && byte_length <= MAX_FIXTURE_BYTES as u64
        && output_sha256 == agenda_sha256
        && output_sha256.len() == 64
        && output_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn contains_nul(value: &str) -> bool {
    value.contains('\0')
}

fn one_email_address(value: &str) -> bool {
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.is_empty()
        && !domain.contains('@')
        && domain.contains('.')
        && !value.chars().any(char::is_whitespace)
}

fn execute_prepare<'a>(
    context: TaskToolExecutionContext<'a>,
    arguments: Value,
) -> TaskToolFuture<'a> {
    Box::pin(async move {
        let request = serde_json::from_value::<PrepareAgendaRequest>(arguments).map_err(|_| {
            "prepare_release_recovery_agenda arguments do not match the registered schema."
                .to_string()
        })?;
        let execution_id = context.execution_id.ok_or_else(|| {
            "Preparing a recovery agenda requires an active approved Task.".to_string()
        })?;
        let task = require_agent_runtime_task(context.persistence, execution_id)?;
        let input = crate::shield_gate::bind_approved_external_file_read(&request.input_path)
            .map_err(|error| error.message)?;
        let input = crate::shield_gate::read_bound_approved_external_file_bounded(
            &input,
            MAX_FIXTURE_BYTES,
        )
        .map_err(|error| error.message)?;
        let milestones =
            serde_json::from_slice::<Vec<MilestoneInput>>(&input.bytes).map_err(|_| {
                "The milestone fixture is not a valid milestone JSON array.".to_string()
            })?;
        let today = Local::now().date_naive();
        let facts = verified_milestone_facts(milestones, today)?;
        let slot = system_calendar_event::find_next_weekday_conflict_free_slot().await?;
        let agenda_items = recovery_agenda_items(&facts);
        let proposed_time = proposed_time(&slot.start_date, &slot.end_date, &slot.time_zone)?;
        let event_notes = shared_agenda_text(&proposed_time, &agenda_items);
        let content = agenda_markdown(
            today,
            &facts,
            &proposed_time,
            &slot.start_date,
            &slot.end_date,
            &slot.time_zone,
            &agenda_items,
        );
        let output = create_exact_new_file(&request.output_path, content.as_bytes())?;
        let mail_body = mail_body_text(
            &proposed_time,
            &agenda_items,
            &output.canonical_path.to_string_lossy(),
        );
        let receipt = AgendaReceipt {
            status: "completed".to_string(),
            verified: true,
            input_path: input.canonical_path.display().to_string(),
            input_sha256: input.sha256,
            output_path: output.canonical_path.display().to_string(),
            output_sha256: output.sha256.clone(),
            byte_length: output.byte_length,
            as_of_date: today.to_string(),
            start_date: slot.start_date,
            end_date: slot.end_date,
            time_zone: slot.time_zone,
            proposed_time,
            agenda_items,
            milestone_facts: facts,
            event_notes,
            mail_body,
        };
        record_event(
            context.persistence,
            &task.task_run_id,
            "release_recovery.agenda_created",
            EvidenceClass::VerifiedPostcondition,
            json!({
                "inputPath": receipt.input_path,
                "inputSha256": receipt.input_sha256,
                "outputPath": receipt.output_path,
                "outputSha256": receipt.output_sha256,
                "startDate": receipt.start_date,
                "endDate": receipt.end_date,
                "agendaItemCount": receipt.agenda_items.len(),
            }),
        )?;
        Ok(ExecuteCommandResponse {
            operation: PREPARE_OPERATION.to_string(),
            status: CommandStatus::Completed,
            message: serde_json::to_string(&receipt).map_err(|error| error.to_string())?,
            metrics: None,
            claims: vec![agenda_evidence_claim(&receipt)],
            verified: true,
            model_used: None,
        })
    })
}

fn agenda_evidence_claim(receipt: &AgendaReceipt) -> String {
    format!(
        "CLAIM release_recovery_agenda_verified=true output_sha256={} input_sha256={} path_sha256={} agenda_item_count=5 start_date={} end_date={}",
        receipt.output_sha256,
        receipt.input_sha256,
        crate::foundation::digest::sha256_hex(receipt.output_path.as_bytes()),
        receipt.start_date,
        receipt.end_date
    )
}

fn verified_milestone_facts(
    milestones: Vec<MilestoneInput>,
    today: NaiveDate,
) -> Result<Vec<MilestoneFact>, String> {
    if milestones.is_empty() || milestones.len() > 256 {
        return Err("The milestone fixture must contain between 1 and 256 milestones.".to_string());
    }
    let mut ids = std::collections::HashSet::new();
    milestones
        .into_iter()
        .map(|milestone| {
            let milestone_id = milestone.milestone_id.trim().to_string();
            let name = milestone.name.trim().to_string();
            let status = milestone.status.trim().to_ascii_uppercase();
            let owner = milestone.owner.trim().to_string();
            let target = NaiveDate::parse_from_str(milestone.target_date.trim(), "%Y-%m-%d")
                .map_err(|_| format!("Milestone {milestone_id} has an invalid target date."))?;
            if milestone_id.is_empty()
                || name.is_empty()
                || owner.is_empty()
                || [
                    milestone_id.as_str(),
                    name.as_str(),
                    status.as_str(),
                    owner.as_str(),
                ]
                .iter()
                .any(|value| contains_nul(value) || value.chars().count() > 512)
                || !ids.insert(milestone_id.clone())
            {
                return Err(
                    "The milestone fixture contains an invalid or duplicate milestone.".to_string(),
                );
            }
            let completed = status == "COMPLETED";
            Ok(MilestoneFact {
                milestone_id,
                name,
                target_date: target.to_string(),
                status,
                owner,
                completed,
                overdue: !completed && target < today,
                unfinished: !completed,
            })
        })
        .collect()
}

fn recovery_agenda_items(facts: &[MilestoneFact]) -> Vec<String> {
    let completed = facts
        .iter()
        .filter(|fact| fact.completed)
        .map(|fact| format!("{} ({})", fact.milestone_id, fact.name))
        .collect::<Vec<_>>();
    let unfinished = facts
        .iter()
        .filter(|fact| fact.unfinished)
        .collect::<Vec<_>>();
    let first = unfinished.first().map_or_else(
        || "Confirm that no unfinished milestone requires recovery action.".to_string(),
        |fact| {
            format!(
                "Decide the recovery date and unblockers for {} ({}), owned by {}.",
                fact.milestone_id, fact.name, fact.owner
            )
        },
    );
    let remaining = if unfinished.len() > 1 {
        unfinished[1..]
            .iter()
            .map(|fact| format!("{} ({}) — {}", fact.milestone_id, fact.name, fact.owner))
            .collect::<Vec<_>>()
            .join("; ")
    } else {
        "No additional unfinished milestones".to_string()
    };
    vec![
        format!(
            "Confirm the completed baseline and any carry-forward work: {}.",
            if completed.is_empty() {
                "no milestones are marked completed".to_string()
            } else {
                completed.join("; ")
            }
        ),
        first,
        format!("Set recovery decisions and dates for the remaining unfinished work: {remaining}."),
        "Resolve cross-milestone dependencies, release blockers, and required handoffs."
            .to_string(),
        "Confirm accountable owners, decisions, and the next verification checkpoints.".to_string(),
    ]
}

fn agenda_markdown(
    today: NaiveDate,
    facts: &[MilestoneFact],
    proposed_time: &str,
    start_date: &str,
    end_date: &str,
    time_zone: &str,
    agenda_items: &[String],
) -> String {
    let rows = facts
        .iter()
        .map(|fact| {
            format!(
                "| {} | {} | {} | {} | {} | {} |",
                fact.milestone_id,
                fact.name.replace('|', "\\|"),
                fact.target_date,
                fact.status,
                fact.owner.replace('|', "\\|"),
                if fact.overdue {
                    "Unfinished and overdue"
                } else if fact.unfinished {
                    "Unfinished"
                } else {
                    "Completed"
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let decisions = facts
        .iter()
        .filter(|fact| fact.unfinished)
        .map(|fact| {
            format!(
                "- {} / {}: confirm recovery date, blockers, decision owner, and next verification evidence.",
                fact.milestone_id, fact.owner
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "# OOMU Release Readiness Recovery Agenda\n\n**Status date:** {today}  \n**Proposed time:** {proposed_time}  \n**Frozen start:** `{start_date}`  \n**Frozen end:** `{end_date}`  \n**Calendar timezone:** `{time_zone}`  \n**Duration:** 30 minutes  \n**Availability:** Tentative\n\n## Milestone facts\n\n| ID | Milestone | Target date | Status | Owner | Recovery assessment |\n|---|---|---|---|---|---|\n{rows}\n\n## Decisions needed\n\n{decisions}\n\n## Agenda — exactly five items\n\n{}\n",
        numbered_items(agenda_items)
    )
}

fn numbered_items(items: &[String]) -> String {
    items
        .iter()
        .enumerate()
        .map(|(index, item)| format!("{}. {item}", index + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

fn shared_agenda_text(proposed_time: &str, items: &[String]) -> String {
    format!(
        "OOMU Release Readiness recovery meeting\n\nProposed time: {proposed_time}\n\nAgenda:\n{}",
        numbered_items(items)
    )
}

fn mail_body_text(proposed_time: &str, items: &[String], output_path: &str) -> String {
    format!(
        "OOMU release readiness recovery meeting\n\nProposed time: {proposed_time}\n\n{}\n\nAgenda file: {output_path}",
        numbered_items(items)
    )
}

fn proposed_time(start: &str, end: &str, time_zone: &str) -> Result<String, String> {
    let start = DateTime::parse_from_rfc3339(start)
        .map_err(|_| "The frozen Calendar start time is invalid.".to_string())?;
    let end = DateTime::parse_from_rfc3339(end)
        .map_err(|_| "The frozen Calendar end time is invalid.".to_string())?;
    Ok(format!(
        "{}–{} ({time_zone})",
        start.format("%A, %B %-d, %Y at %-I:%M %p"),
        end.format("%-I:%M %p")
    ))
}

struct CreatedAgendaFile {
    canonical_path: PathBuf,
    sha256: String,
    byte_length: u64,
}

fn create_exact_new_file(path: &str, bytes: &[u8]) -> Result<CreatedAgendaFile, String> {
    let resolved = crate::shield_gate::validate_approved_external_write_target(path)
        .map_err(|error| error.message)?;
    if fs::symlink_metadata(&resolved).is_ok() {
        return Err(format!(
            "{} already exists. OOMU will not replace it.",
            resolved
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("The agenda file")
        ));
    }
    let parent = resolved
        .parent()
        .ok_or_else(|| "The agenda destination has no safe parent folder.".to_string())?;
    let binding = crate::shield_gate::bind_approved_external_directory_creation(
        &parent.display().to_string(),
    )
    .map_err(|error| error.message)?;
    let parent_existed = binding.existed_when_bound();
    let created_parent = crate::shield_gate::create_bound_approved_external_directory(&binding)
        .map_err(|error| error.message)?;
    if created_parent != parent {
        return Err("The agenda destination changed before OOMU could write it.".to_string());
    }
    #[cfg(unix)]
    let opened = {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&resolved)
    };
    #[cfg(not(unix))]
    let opened = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&resolved);
    let mut output = match opened {
        Ok(output) => output,
        Err(error) => {
            if !parent_existed {
                let _ = fs::remove_dir(&created_parent);
            }
            return Err(format!("OOMU couldn't create the recovery agenda: {error}"));
        }
    };
    if let Err(error) = output.write_all(bytes).and_then(|_| output.sync_all()) {
        drop(output);
        let _ = fs::remove_file(&resolved);
        if !parent_existed {
            let _ = fs::remove_dir(&created_parent);
        }
        return Err(format!("OOMU couldn't finish the recovery agenda: {error}"));
    }
    drop(output);
    let metadata = fs::symlink_metadata(&resolved)
        .map_err(|_| "OOMU created the agenda but could not reopen it.".to_string())?;
    let canonical = fs::canonicalize(&resolved)
        .map_err(|_| "OOMU created the agenda but could not verify its path.".to_string())?;
    let actual = fs::read(&canonical)
        .map_err(|_| "OOMU created the agenda but could not verify its bytes.".to_string())?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || canonical != resolved
        || actual != bytes
    {
        let _ = fs::remove_file(&resolved);
        if !parent_existed {
            let _ = fs::remove_dir(&created_parent);
        }
        return Err(
            "OOMU removed the unverified recovery agenda because its final bytes or path changed."
                .to_string(),
        );
    }
    Ok(CreatedAgendaFile {
        canonical_path: canonical,
        sha256: crate::foundation::digest::sha256_hex(&actual),
        byte_length: metadata.len(),
    })
}

fn resolve_calendar(
    _persistence: &crate::db::PersistenceEngine,
    _execution_id: Option<&str>,
    arguments: Value,
    outputs: &[ExecuteCommandResponse],
) -> Result<Value, String> {
    resolve_calendar_from_outputs(arguments, outputs)
}

fn resolve_calendar_from_outputs(
    arguments: Value,
    outputs: &[ExecuteCommandResponse],
) -> Result<Value, String> {
    let request = serde_json::from_value::<CalendarPlanRequest>(arguments)
        .map_err(|_| "The recovery Calendar plan is invalid.".to_string())?;
    let agenda = agenda_receipt_at(outputs, request.agenda_step)?;
    serde_json::to_value(CalendarResolvedRequest {
        calendar_name: request.calendar_name,
        title: request.title,
        start_date: agenda.start_date,
        end_date: agenda.end_date,
        location: String::new(),
        notes: agenda.event_notes,
        availability: request.availability,
        agenda_step: request.agenda_step,
        agenda_sha256: agenda.output_sha256.clone(),
        output_path: agenda.output_path,
        output_sha256: agenda.output_sha256,
        byte_length: agenda.byte_length,
    })
    .map_err(|error| error.to_string())
}

fn execute_calendar<'a>(
    context: TaskToolExecutionContext<'a>,
    arguments: Value,
) -> TaskToolFuture<'a> {
    Box::pin(async move {
        let request = serde_json::from_value::<CalendarResolvedRequest>(arguments)
            .map_err(|_| "The resolved recovery Calendar request is invalid.".to_string())?;
        verify_resolved_agenda_binding(
            &request.output_path,
            &request.output_sha256,
            request.byte_length,
        )
        .map_err(|message| agenda_binding_changed_error(CALENDAR_OPERATION, message))?;
        system_calendar_event::execute_exact_conflict_checked_registration(
            context,
            calendar_base_arguments(&request),
            CALENDAR_OPERATION,
        )
        .await
    })
}

fn calendar_base_arguments(request: &CalendarResolvedRequest) -> Value {
    json!({
        "calendarName":request.calendar_name,
        "title":request.title,
        "startDate":request.start_date,
        "endDate":request.end_date,
        "location":request.location,
        "notes":request.notes,
        "availability":request.availability,
    })
}

fn resolve_mail(
    _persistence: &crate::db::PersistenceEngine,
    _execution_id: Option<&str>,
    arguments: Value,
    outputs: &[ExecuteCommandResponse],
) -> Result<Value, String> {
    resolve_mail_from_outputs(arguments, outputs)
}

fn resolve_mail_from_outputs(
    arguments: Value,
    outputs: &[ExecuteCommandResponse],
) -> Result<Value, String> {
    let request = serde_json::from_value::<MailPlanRequest>(arguments)
        .map_err(|_| "The recovery Mail plan is invalid.".to_string())?;
    let agenda = agenda_receipt_at(outputs, request.agenda_step)?;
    let calendar = output_at(outputs, request.calendar_step, CALENDAR_OPERATION)?;
    let calendar_receipt = serde_json::from_str::<Value>(&calendar.message)
        .map_err(|_| "The recovery Calendar receipt is invalid.".to_string())?;
    let expected_notes_sha256 =
        crate::foundation::digest::sha256_hex(agenda.event_notes.as_bytes());
    if calendar_receipt.get("verified").and_then(Value::as_bool) != Some(true)
        || calendar_receipt.get("exists").and_then(Value::as_bool) != Some(true)
        || calendar_receipt.get("startDate").and_then(Value::as_str)
            != Some(agenda.start_date.as_str())
        || calendar_receipt.get("endDate").and_then(Value::as_str) != Some(agenda.end_date.as_str())
        || calendar_receipt.get("notesSha256").and_then(Value::as_str)
            != Some(expected_notes_sha256.as_str())
    {
        return Err(
            "The verified Calendar receipt no longer matches the frozen recovery agenda."
                .to_string(),
        );
    }
    serde_json::to_value(MailResolvedRequest {
        to: request.to,
        subject: request.subject,
        body: agenda.mail_body,
        start_date: agenda.start_date,
        end_date: agenda.end_date,
        agenda_items: agenda.agenda_items,
        agenda_step: request.agenda_step,
        calendar_step: request.calendar_step,
        agenda_sha256: agenda.output_sha256.clone(),
        output_path: agenda.output_path,
        output_sha256: agenda.output_sha256,
        byte_length: agenda.byte_length,
    })
    .map_err(|error| error.to_string())
}

fn execute_mail<'a>(context: TaskToolExecutionContext<'a>, arguments: Value) -> TaskToolFuture<'a> {
    Box::pin(async move {
        let request = serde_json::from_value::<MailResolvedRequest>(arguments)
            .map_err(|_| "The resolved recovery Mail request is invalid.".to_string())?;
        verify_resolved_agenda_binding(
            &request.output_path,
            &request.output_sha256,
            request.byte_length,
        )
        .map_err(|message| agenda_binding_changed_error(MAIL_OPERATION, message))?;
        let mut response =
            system_mail::execute_registration(context, mail_base_arguments(&request)).await?;
        response.operation = MAIL_OPERATION.to_string();
        Ok(response)
    })
}

fn mail_base_arguments(request: &MailResolvedRequest) -> Value {
    json!({
        "to":request.to,
        "subject":request.subject,
        "body":request.body,
    })
}

fn agenda_receipt_at(
    outputs: &[ExecuteCommandResponse],
    index: usize,
) -> Result<AgendaReceipt, String> {
    let output = output_at(outputs, index, PREPARE_OPERATION)?;
    if output.message.len() > MAX_RECEIPT_BYTES {
        return Err("The recovery agenda receipt is too large.".to_string());
    }
    let receipt = serde_json::from_str::<AgendaReceipt>(&output.message)
        .map_err(|_| "The recovery agenda receipt is invalid.".to_string())?;
    if receipt.status != "completed"
        || !receipt.verified
        || receipt.agenda_items.len() != EXPECTED_AGENDA_ITEMS
        || receipt.output_sha256.len() != 64
    {
        return Err("The recovery agenda receipt is not verified.".to_string());
    }
    verify_agenda_receipt_file(&receipt)?;
    Ok(receipt)
}

/// Reopens the exact agenda at every receipt-consumption boundary. Calendar
/// and Mail therefore cannot act on a stale checkpoint if the file was moved,
/// replaced, truncated, or edited after its original receipt was recorded.
fn verify_agenda_receipt_file(receipt: &AgendaReceipt) -> Result<(), String> {
    let bytes = verify_resolved_agenda_binding(
        &receipt.output_path,
        &receipt.output_sha256,
        receipt.byte_length,
    )?;
    let status_date = NaiveDate::parse_from_str(&receipt.as_of_date, "%Y-%m-%d")
        .map_err(|_| "The recovery agenda receipt has an invalid status date.".to_string())?;
    let expected_markdown = agenda_markdown(
        status_date,
        &receipt.milestone_facts,
        &receipt.proposed_time,
        &receipt.start_date,
        &receipt.end_date,
        &receipt.time_zone,
        &receipt.agenda_items,
    );
    let expected_event_notes = shared_agenda_text(&receipt.proposed_time, &receipt.agenda_items);
    let expected_mail_body = mail_body_text(
        &receipt.proposed_time,
        &receipt.agenda_items,
        &receipt.output_path,
    );
    if bytes != expected_markdown.as_bytes()
        || receipt.event_notes != expected_event_notes
        || receipt.mail_body != expected_mail_body
    {
        return Err(
            "The recovery agenda receipt no longer matches the exact saved agenda semantics."
                .to_string(),
        );
    }
    Ok(())
}

fn verify_resolved_agenda_binding(
    output_path: &str,
    output_sha256: &str,
    byte_length: u64,
) -> Result<Vec<u8>, String> {
    let path = Path::new(output_path);
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| "The verified recovery agenda is no longer available.".to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("The verified recovery agenda is no longer a regular file.".to_string());
    }
    let canonical = fs::canonicalize(path)
        .map_err(|_| "The recovery agenda path can no longer be verified.".to_string())?;
    if canonical != path {
        return Err("The recovery agenda path changed after it was verified.".to_string());
    }
    let bytes = fs::read(&canonical)
        .map_err(|_| "The recovery agenda can no longer be reopened safely.".to_string())?;
    if metadata.len() != byte_length
        || crate::foundation::digest::sha256_hex(&bytes) != output_sha256
    {
        return Err(
            "The recovery agenda changed after it was verified. Calendar and Mail were not changed."
                .to_string(),
        );
    }
    Ok(bytes)
}

fn agenda_binding_changed_error(operation: &str, message: String) -> String {
    let code = if operation == CALENDAR_OPERATION {
        "calendar_agenda_binding_changed"
    } else {
        "mail_agenda_binding_changed"
    };
    json!({
        "taskToolError": {
            "code": code,
            "message": message,
            "context": {
                "failurePhase": "preflight",
                "changedState": false,
            }
        }
    })
    .to_string()
}

fn output_at<'a>(
    outputs: &'a [ExecuteCommandResponse],
    index: usize,
    operation: &str,
) -> Result<&'a ExecuteCommandResponse, String> {
    let output = outputs
        .get(index)
        .ok_or_else(|| format!("The required {operation} receipt is missing."))?;
    if output.operation != operation || output.status.as_str() != "completed" || !output.verified {
        return Err(format!("The required {operation} receipt is not verified."));
    }
    Ok(output)
}

pub(crate) async fn verify_postcondition(
    calendar_plan_arguments: Value,
    mail_plan_arguments: Value,
    originally_requested_calendar: String,
    outputs: &[ExecuteCommandResponse],
    app: &tauri::AppHandle,
) -> Result<Value, String> {
    if outputs.len() != 3 {
        return Err("The recovery workflow does not have exactly three receipts.".to_string());
    }
    let agenda = agenda_receipt_at(outputs, 0)?;
    let calendar_resolved = resolve_calendar_from_outputs(calendar_plan_arguments, outputs)?;
    let calendar_request = serde_json::from_value::<CalendarResolvedRequest>(calendar_resolved)
        .map_err(|_| "The final recovery Calendar request is invalid.".to_string())?;
    let calendar_evidence = system_calendar_event::verify_exact_event_postcondition(
        calendar_base_arguments(&calendar_request),
        &output_at(outputs, 1, CALENDAR_OPERATION)?.message,
        &originally_requested_calendar,
    )
    .await?;
    let mail_resolved = resolve_mail_from_outputs(mail_plan_arguments, outputs)?;
    let mail_request = serde_json::from_value::<MailResolvedRequest>(mail_resolved)
        .map_err(|_| "The final recovery Mail request is invalid.".to_string())?;
    let mail_evidence = system_mail::verify_exact_draft_postcondition(
        app,
        mail_base_arguments(&mail_request),
        &output_at(outputs, 2, MAIL_OPERATION)?.message,
    )
    .await?;
    Ok(json!({
        "verified":true,
        "fileSha256":agenda.output_sha256,
        "inputSha256":agenda.input_sha256,
        "startDate":agenda.start_date,
        "endDate":agenda.end_date,
        "agendaItemCount":agenda.agenda_items.len(),
        "calendar":calendar_evidence,
        "mail":mail_evidence,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agenda_receipt_for_file(path: &Path) -> (AgendaReceipt, Vec<u8>) {
        let status_date = NaiveDate::from_ymd_opt(2026, 7, 20).unwrap();
        let proposed = "Tuesday, July 21, 2026 at 1:30 PM–2:00 PM (America/New_York)";
        let items = (1..=5)
            .map(|index| format!("Agenda item {index}"))
            .collect::<Vec<_>>();
        let facts = Vec::new();
        let bytes = agenda_markdown(
            status_date,
            &facts,
            proposed,
            "2026-07-21T13:30:00-04:00",
            "2026-07-21T14:00:00-04:00",
            "America/New_York",
            &items,
        )
        .into_bytes();
        let event_notes = shared_agenda_text(proposed, &items);
        let mail_body = mail_body_text(proposed, &items, &path.display().to_string());
        let receipt = AgendaReceipt {
            status: "completed".to_string(),
            verified: true,
            input_path: "/tmp/milestones.json".to_string(),
            input_sha256: "a".repeat(64),
            output_path: path.display().to_string(),
            output_sha256: crate::foundation::digest::sha256_hex(&bytes),
            byte_length: bytes.len() as u64,
            as_of_date: "2026-07-20".to_string(),
            start_date: "2026-07-21T13:30:00-04:00".to_string(),
            end_date: "2026-07-21T14:00:00-04:00".to_string(),
            time_zone: "America/New_York".to_string(),
            proposed_time: proposed.to_string(),
            agenda_items: items,
            milestone_facts: facts,
            event_notes,
            mail_body,
        };
        (receipt, bytes)
    }

    #[test]
    fn agenda_evidence_claim_hashes_paths_with_spaces() {
        let path = Path::new("/tmp/OOMU Scenario 2/release recovery agenda.md");
        let (receipt, _) = agenda_receipt_for_file(path);
        let claim = agenda_evidence_claim(&receipt);
        assert!(claim.starts_with("CLAIM release_recovery_agenda_verified=true "));
        assert!(claim.contains(&format!(
            "path_sha256={}",
            crate::foundation::digest::sha256_hex(receipt.output_path.as_bytes())
        )));
        assert!(!claim.contains(&receipt.output_path));
        assert!(!claim.contains(" path="));
    }

    #[test]
    fn agenda_items_are_always_exactly_five_and_preserve_fixture_facts() {
        let facts = verified_milestone_facts(
            vec![
                MilestoneInput {
                    milestone_id: "M1".to_string(),
                    name: "Core".to_string(),
                    target_date: "2026-07-06".to_string(),
                    status: "COMPLETED".to_string(),
                    owner: "Alex".to_string(),
                },
                MilestoneInput {
                    milestone_id: "M2".to_string(),
                    name: "Localization".to_string(),
                    target_date: "2026-07-10".to_string(),
                    status: "IN_PROGRESS".to_string(),
                    owner: "Alex".to_string(),
                },
                MilestoneInput {
                    milestone_id: "M3".to_string(),
                    name: "Validation".to_string(),
                    target_date: "2026-07-15".to_string(),
                    status: "PENDING".to_string(),
                    owner: "OOMU".to_string(),
                },
            ],
            NaiveDate::from_ymd_opt(2026, 7, 20).unwrap(),
        )
        .unwrap();
        let items = recovery_agenda_items(&facts);
        assert_eq!(items.len(), 5);
        assert!(items.join("\n").contains("M1"));
        assert!(items.join("\n").contains("M2"));
        assert!(items.join("\n").contains("M3"));
        assert!(facts[1].overdue && facts[2].overdue);
        assert!(facts[0].completed && !facts[0].overdue);
    }

    #[test]
    fn planned_and_resolved_schemas_are_distinct_and_strict() {
        assert!(validate_calendar_plan(json!({
            "calendarName":"OOMU Test",
            "title":"OOMU Release Readiness",
            "agendaStep":0,
            "availability":"tentative"
        }))
        .is_ok());
        assert!(validate_calendar_plan(json!({
            "calendarName":"OOMU Test",
            "title":"OOMU Release Readiness",
            "agendaStep":0,
            "availability":"tentative",
            "startDate":"2026-07-21T13:30:00-04:00"
        }))
        .is_err());
        assert!(validate_mail_plan(json!({
            "to":"tester@example.com",
            "subject":"OOMU Release Readiness — Recovery Meeting",
            "agendaStep":0,
            "calendarStep":1
        }))
        .is_ok());
        let binding = json!({
            "calendarName":"OOMU Test",
            "title":"OOMU Release Readiness",
            "startDate":"2026-07-21T13:30:00-04:00",
            "endDate":"2026-07-21T14:00:00-04:00",
            "location":"",
            "notes":"Verified notes",
            "availability":"tentative",
            "agendaStep":0,
            "agendaSha256":"a".repeat(64),
            "outputPath":"/tmp/release_recovery_agenda.md",
            "outputSha256":"a".repeat(64),
            "byteLength":1024
        });
        assert!(validate_calendar_resolved(binding.clone()).is_ok());
        let mut mismatched = binding;
        mismatched["outputSha256"] = json!("b".repeat(64));
        assert!(validate_calendar_resolved(mismatched).is_err());
    }

    #[test]
    fn downstream_receipt_consumers_reopen_and_rehash_the_agenda() {
        let root = std::env::temp_dir().join(format!(
            "oomu-release-recovery-agenda-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let path = root.join("agenda.md");
        let canonical = root.canonicalize().unwrap().join("agenda.md");
        let (receipt, original) = agenda_receipt_for_file(&canonical);
        fs::write(&path, &original).unwrap();
        assert!(verify_agenda_receipt_file(&receipt).is_ok());

        let mut divergent = receipt.clone();
        divergent.event_notes = "Different event notes".to_string();
        assert!(verify_agenda_receipt_file(&divergent)
            .unwrap_err()
            .contains("saved agenda semantics"));

        fs::write(&path, b"# Changed agenda\n").unwrap();
        let error = verify_agenda_receipt_file(&receipt).unwrap_err();
        assert!(error.contains("changed after it was verified"));
        fs::remove_file(path).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
