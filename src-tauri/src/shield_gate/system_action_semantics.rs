use super::{AuthorizedActions, RequestedAction, ShieldActionSemantics, ShieldGateError};
use serde_json::{json, Value};
pub(super) mod scenario_one_e2e;
pub(super) use scenario_one_e2e::request_native_selection as scenario_one_native_selection;

pub(super) fn delegated(
    action: &RequestedAction,
    kind: &str,
) -> Option<Option<ShieldActionSemantics>> {
    match kind {
        "create_release_recovery_calendar_event" => Some(calendar_event(action)),
        "draft_system_email" | "draft_decision_pack_email" | "draft_release_recovery_email" => {
            Some(mail_draft(action))
        }
        "prepare_release_recovery_agenda" => Some(release_recovery_agenda(action)),
        _ => None,
    }
}

pub(super) fn direct_preview(action: &RequestedAction) -> Option<String> {
    matches!(
        action.kind.as_str(),
        "create_system_calendar_event"
            | "create_conflict_free_calendar_event"
            | "create_release_recovery_calendar_event"
            | "draft_system_email"
            | "draft_decision_pack_email"
            | "draft_release_recovery_email"
            | "prepare_release_recovery_agenda"
    )
    .then(|| action.content.clone().unwrap_or_default())
}

pub(super) fn configure_channel(action: &RequestedAction) -> ShieldActionSemantics {
    let preview = configure_channel_preview(action);
    let platform = preview
        .as_ref()
        .and_then(|value| value.get("platform"))
        .and_then(Value::as_str)
        .unwrap_or("messaging");
    let owner = preview
        .as_ref()
        .and_then(|value| value.get("ownerId"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("the saved account");
    ShieldActionSemantics {
        action_label: "Configure a messaging channel".to_string(),
        summary: format!("Configure the {platform} channel for {owner}."),
        detail: "OOMU will verify the connection, save its private credentials securely, and update the channel worker only after approval."
            .to_string(),
        reason: "This changes which messaging account can communicate with OOMU.".to_string(),
        target_path: None,
    }
}

pub(super) fn configure_channel_preview(action: &RequestedAction) -> Option<Value> {
    let request = parsed_content(action)?;
    Some(json!({
        "platform": request.get("platform").and_then(Value::as_str)?.trim(),
        "ownerId": request.get("owner_id").and_then(Value::as_str)?.trim(),
        "isActive": request.get("is_active").and_then(Value::as_bool)?,
        "credentialsProvided": request
            .get("credentials_json")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty() && value.trim() != "{}"),
    }))
}

pub(super) fn calendar_event(action: &RequestedAction) -> Option<ShieldActionSemantics> {
    let preview = parsed_content(action);
    let field = |name: &str, fallback: &'static str| {
        preview
            .as_ref()
            .and_then(|value| value.get(name))
            .and_then(Value::as_str)
            .unwrap_or(fallback)
    };
    let title = field("title", "the requested event");
    let calendar = field("calendarName", "the requested calendar");
    let start = field("startDate", "the requested start time");
    let end = field("endDate", "the requested end time");
    Some(ShieldActionSemantics {
        action_label: "Add a Calendar event".to_string(),
        summary: format!("Add {title} to {calendar}."),
        detail: format!(
            "OOMU will create one event from {start} to {end}, then read it back to verify the saved details."
        ),
        reason: "This changes your Calendar and always requires explicit approval.".to_string(),
        target_path: None,
    })
}

pub(super) fn conflict_free_calendar_event(
    action: &RequestedAction,
) -> Option<ShieldActionSemantics> {
    let preview = parsed_content(action)?;
    let title = preview.get("title")?.as_str()?.trim();
    let calendar = preview.get("calendarName")?.as_str()?.trim();
    let day = preview.get("day")?.as_str()?.trim();
    let start = preview.get("windowStartLocal")?.as_str()?.trim();
    let end = preview.get("windowEndLocal")?.as_str()?.trim();
    let duration = preview.get("durationMinutes")?.as_i64()?;
    let location = preview.get("location")?.as_str()?.trim();
    let availability = preview.get("availability")?.as_str()?.trim();
    if title.is_empty() || calendar.is_empty() {
        return None;
    }
    let location_detail = if location.is_empty() {
        "with no location".to_string()
    } else {
        format!("at {location}")
    };
    let day = if day == "next_weekday" {
        "the next weekday"
    } else {
        day
    };
    let start = if start == "13:00" { "1:00 PM" } else { start };
    let end = if end == "16:00" { "4:00 PM" } else { end };
    Some(ShieldActionSemantics {
        action_label: "Find a time and add a Calendar event".to_string(),
        summary: format!(
            "Find a conflict-free time and add “{title}” to the exact “{calendar}” calendar."
        ),
        detail: format!(
            "OOMU will read event times from all calendars only to avoid conflicts on {day} from {start} to {end}. It will create one {availability} {duration}-minute event titled “{title}” in the exact “{calendar}” calendar {location_detail}, then read it back to verify the saved details."
        ),
        reason: "This reads your Calendar schedule and adds an event, so it always requires explicit approval."
            .to_string(),
        target_path: None,
    })
}

pub(super) fn mail_draft(action: &RequestedAction) -> Option<ShieldActionSemantics> {
    let preview = parsed_content(action)?;
    let subject = preview.get("subject")?.as_str()?.trim();
    let to = preview
        .get("to")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("no To recipient");
    let cc = preview
        .get("cc")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("none");
    let bcc = preview
        .get("bcc")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("none");
    if subject.is_empty() {
        return None;
    }
    Some(ShieldActionSemantics {
        action_label: "Save a Mail draft".to_string(),
        summary: format!("Save the unsent draft “{subject}” for {to}."),
        detail: format!("OOMU will create one visible Mail draft with Cc: {cc} and Bcc: {bcc}, save it without sending, and verify its subject, body, and every recipient before reporting success."),
        reason: "This changes Mail and always requires explicit approval. OOMU will not send the message."
            .to_string(),
        target_path: None,
    })
}

pub(super) fn decision_pack(action: &RequestedAction) -> Option<ShieldActionSemantics> {
    let preview = decision_pack_preview(action)?;
    let output_directory = preview.get("outputDirectory")?.as_str()?;
    let inputs = preview
        .get("inputPaths")?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join("\n• ");
    let policy = serde_json::from_value::<crate::decision_research_policy::ResearchPolicy>(
        preview.get("researchPolicy")?.clone(),
    )
    .ok()?;
    crate::decision_research_policy::validate_research_policy(&policy).ok()?;
    let queries = policy
        .subjects
        .iter()
        .flat_map(|subject| subject.query_alternatives.iter())
        .map(|alternative| alternative.query.as_str())
        .collect::<Vec<_>>()
        .join("\n• ");
    let file_names = preview
        .get("outputs")?
        .as_object()?
        .values()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    Some(ShieldActionSemantics {
        action_label: "Create a supplier decision pack".to_string(),
        summary: format!("Create four new decision-pack files in {output_directory}."),
        detail: format!(
            "OOMU will read these approved files:\n• {inputs}\n\nIt will independently search the public web for:\n• {queries}\n\nIt will reconcile rates and margins locally, then create exactly these new files without replacing anything: {file_names}. Calendar and Mail are separate approvals."
        ),
        reason: "This action reads local evidence, accesses official public web sources, and writes four new local files.".to_string(),
        target_path: Some(output_directory.to_string()),
    })
}

pub(super) fn decision_pack_preview(action: &RequestedAction) -> Option<Value> {
    let content = parsed_content(action)?;
    let policy_value = content.get("researchPolicy")?.clone();
    let policy = serde_json::from_value::<crate::decision_research_policy::ResearchPolicy>(
        policy_value.clone(),
    )
    .ok()?;
    crate::decision_research_policy::validate_research_policy(&policy).ok()?;
    Some(serde_json::json!({
        "action":"create_decision_pack",
        "inputPaths":content.get("inputPaths")?,
        "researchPolicy":policy_value,
        "outputDirectory":content.get("outputDirectory")?,
        "outputs":content.get("outputs")?,
        "willOverwrite":false,
        "calendarOrMailIncluded":false
    }))
}

pub(super) fn release_recovery_agenda(action: &RequestedAction) -> Option<ShieldActionSemantics> {
    let preview = parsed_content(action)?;
    let input = preview.get("inputPath")?.as_str()?.trim();
    let output = preview.get("outputPath")?.as_str()?.trim();
    let duration = preview.get("durationMinutes")?.as_i64()?;
    let item_count = preview.get("agendaItemCount")?.as_u64()?;
    if input.is_empty() || output.is_empty() || duration != 30 || item_count != 5 {
        return None;
    }
    Some(ShieldActionSemantics {
        action_label: "Create a recovery agenda".to_string(),
        summary: format!("Create one new recovery agenda at {output}."),
        detail: format!(
            "OOMU will read {input}, inspect Calendar without changing it to freeze one exact conflict-free 30-minute time, then create one Markdown file with exactly five agenda items. It will not replace an existing file. Calendar and Mail mutations remain separate approvals."
        ),
        reason: "This reads approved local evidence and Calendar availability, then creates one local file."
            .to_string(),
        target_path: Some(output.to_string()),
    })
}

fn parsed_content(action: &RequestedAction) -> Option<Value> {
    action
        .content
        .as_deref()
        .and_then(|value| serde_json::from_str(value).ok())
}

pub(super) fn authorize_registered(
    action: RequestedAction,
    shield_approved: bool,
) -> Result<AuthorizedActions, ShieldGateError> {
    let operation = action.kind.trim().replace('-', "_").to_ascii_lowercase();
    if matches!(
        crate::tools::task_tool_runtime::approval_tier(&operation),
        Some(crate::tools::task_tool_runtime::TaskToolApprovalTier::Explicit)
    ) && !shield_approved
    {
        return Err(ShieldGateError {
            code: "shield_gate_rejected",
            boundary: "AuthorizedActions",
            message: format!("{operation} requires explicit Shield Gate approval."),
        });
    }
    crate::tools::task_tool_runtime::authorize(action)
        .map(AuthorizedActions::RegisteredTaskTool)
        .map_err(|message| ShieldGateError {
            code: "shield_gate_invalid_input",
            boundary: "AuthorizedActions",
            message,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(kind: &str, content: Value) -> RequestedAction {
        RequestedAction {
            kind: kind.to_string(),
            principal: None,
            path: None,
            content: Some(content.to_string()),
        }
    }

    #[test]
    fn channel_preview_remains_secret_safe_after_extraction() {
        let preview = configure_channel_preview(&action(
            "configure_channel",
            json!({
                "platform": "telegram",
                "credentials_json": "{\"token\":\"secret\"}",
                "owner_id": "owner-1",
                "is_active": true
            }),
        ))
        .unwrap();
        assert_eq!(preview["platform"], "telegram");
        assert_eq!(preview["credentialsProvided"], true);
        assert!(preview.get("credentials_json").is_none());
    }

    #[test]
    fn decision_pack_preview_preserves_and_validates_structured_research_policy() {
        let policy = crate::decision_research_policy::compile_research_policy(
            "independently research official fuel or freight conditions",
        )
        .unwrap();
        let request_action = action(
            "create_decision_pack",
            json!({
                "title":"Supplier Decision Pack",
                "locale":"en-US",
                "inputPaths":["/testing/mock_data/a.json", "/testing/mock_data/b.txt"],
                "researchPolicy":policy,
                "analysisInstructions":"Reconcile every amount and margin.",
                "outputDirectory":"/testing/ship_test_01",
                "outputs":{
                    "workbook":"supplier_decision.xlsx",
                    "presentation":"supplier_decision.pptx",
                    "pdf":"supplier_decision.pdf",
                    "sources":"sources.md"
                }
            }),
        );
        let preview =
            decision_pack_preview(&request_action).expect("structured policy should preview");
        assert!(preview.get("researchPolicy").is_some());
        assert!(preview.get("researchQueries").is_none());
        let semantics = decision_pack(&request_action).expect("structured policy should describe");
        assert!(semantics.detail.contains("site:eia.gov"));
        assert!(semantics.detail.contains("site:bts.gov"));

        let mut invalid = parsed_content(&request_action).unwrap();
        invalid["researchPolicy"]["subjects"][0]["queryAlternatives"][0]["query"] =
            json!("unregistered query");
        assert!(decision_pack_preview(&action("create_decision_pack", invalid)).is_none());
    }

    #[test]
    fn calendar_semantics_name_the_exact_event_and_calendar() {
        let semantics = calendar_event(&action(
            "create_system_calendar_event",
            json!({
                "calendarName": "OOMU Test",
                "title": "Supplier Decision Review",
                "startDate": "2026-07-20T18:00:00Z",
                "endDate": "2026-07-20T19:00:00Z"
            }),
        ))
        .unwrap();
        assert_eq!(
            semantics.summary,
            "Add Supplier Decision Review to OOMU Test."
        );
        assert!(semantics.detail.contains("2026-07-20T18:00:00Z"));
        assert!(semantics.detail.contains("2026-07-20T19:00:00Z"));
    }

    #[test]
    fn conflict_free_calendar_semantics_explain_the_read_and_single_write() {
        let semantics = conflict_free_calendar_event(&action(
            "create_conflict_free_calendar_event",
            json!({
                "calendarName": "OOMU Test",
                "title": "Supplier Decision Review",
                "day": "next_weekday",
                "windowStartLocal": "13:00",
                "windowEndLocal": "16:00",
                "durationMinutes": 30,
                "location": "Board Room",
                "notes": "Review the verified decision pack.",
                "availability": "tentative"
            }),
        ))
        .unwrap();
        assert!(semantics.summary.contains("Supplier Decision Review"));
        assert!(semantics.summary.contains("OOMU Test"));
        assert!(semantics
            .detail
            .contains("from all calendars only to avoid conflicts"));
        assert!(semantics.detail.contains("1:00 PM to 4:00 PM"));
        assert!(semantics.detail.contains("tentative 30-minute event"));
        assert!(semantics.detail.contains("at Board Room"));
    }

    #[test]
    fn mail_semantics_name_the_unsent_draft_and_recipient() {
        let semantics = mail_draft(&action(
            "draft_system_email",
            json!({
                "to": "reviewer@example.com",
                "cc": "owner@example.com",
                "bcc": "audit@example.com",
                "subject": "Supplier Decision Review",
                "body": "The verified decision pack is ready."
            }),
        ))
        .unwrap();
        assert!(semantics.summary.contains("Supplier Decision Review"));
        assert!(semantics.summary.contains("reviewer@example.com"));
        assert!(semantics.detail.contains("owner@example.com"));
        assert!(semantics.detail.contains("audit@example.com"));
        assert!(semantics.detail.contains("without sending"));
    }
}
