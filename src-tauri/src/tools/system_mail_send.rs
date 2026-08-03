use crate::{
    native_app_ports::{LocalApplicationMailPort, LocalMailReceipt, MailSendRequest},
    p0_contracts::EvidenceClass,
    shield_gate::{CommandStatus, ExecuteCommandResponse},
    tools::{
        task_runtime::{record_event, require_agent_runtime_task},
        task_tool_runtime::{
            TaskToolApprovalTier, TaskToolExecutionContext, TaskToolFuture, TaskToolMetadata,
            TaskToolRegistration, TaskToolRiskTier, TaskToolValidation,
        },
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::{fs, path::Path};

const OPERATION: &str = "send_system_email";
const MAX_RECIPIENT_TEXT_CHARS: usize = 4_096;
const MAX_SUBJECT_CHARS: usize = 998;
const MAX_BODY_CHARS: usize = 20_000;
const MAX_ATTACHMENT_PATH_CHARS: usize = 4_096;
const MAX_ATTACHMENT_BYTES: u64 = 25 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SendSystemEmailRequest {
    to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bcc: Option<String>,
    subject: String,
    body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    attachment_path: Option<String>,
}

fn local_mail_send_request(request: &SendSystemEmailRequest) -> MailSendRequest {
    MailSendRequest {
        to: request.to.clone(),
        cc: request.cc.clone(),
        bcc: request.bcc.clone(),
        subject: request.subject.clone(),
        body: request.body.clone(),
        attachment_path: request.attachment_path.clone(),
    }
}

#[derive(Clone, Debug)]
struct VerifiedMailAttachment {
    canonical_path: String,
    file_name: String,
    sha256: String,
    byte_length: u64,
}

pub(crate) fn register_task_tool() -> Result<(), String> {
    crate::tools::task_tool_runtime::register(TaskToolRegistration {
        operation: OPERATION,
        validate: validate_registration,
        validate_resolved: validate_registration,
        resolve: crate::tools::task_tool_runtime::identity_resolver,
        execute: execute_registration,
        planner_context: None,
        schema: input_schema,
        metadata: TaskToolMetadata {
            description: "Send one exact email through macOS Mail and verify exactly one matching message in Sent Mail. This is not a draft action.",
            risk_tier: TaskToolRiskTier::SystemExec,
            approval_tier: TaskToolApprovalTier::Explicit,
            agent_error_code: "mail_send_failed",
            agent_error_boundary: "SystemMailSend",
            execution_path: "After explicit Shield approval, the trusted built-in Mail bridge sent one exact message and verified a unique matching Sent Mail record.",
        },
    })
}

fn input_schema() -> Value {
    json!({
        "type":"object",
        "properties":{
            "to":{"type":"string","minLength":3,"maxLength":MAX_RECIPIENT_TEXT_CHARS,"description":"Comma-separated To recipients."},
            "cc":{"type":"string","maxLength":MAX_RECIPIENT_TEXT_CHARS},
            "bcc":{"type":"string","maxLength":MAX_RECIPIENT_TEXT_CHARS},
            "subject":{"type":"string","minLength":1,"maxLength":MAX_SUBJECT_CHARS},
            "body":{"type":"string","minLength":1,"maxLength":MAX_BODY_CHARS},
            "attachmentPath":{"type":"string","minLength":1,"maxLength":MAX_ATTACHMENT_PATH_CHARS,"description":"Optional exact verified local report to attach."}
        },
        "required":["to","subject","body"],
        "additionalProperties":false
    })
}

fn validate_registration(arguments: Value) -> Result<TaskToolValidation, String> {
    let mut request =
        serde_json::from_value::<SendSystemEmailRequest>(arguments).map_err(|_| {
            "send_system_email arguments do not match the registered schema.".to_string()
        })?;
    request.to = normalize_recipients(Some(request.to), "to")?
        .ok_or_else(|| "send_system_email requires at least one To recipient.".to_string())?;
    request.cc = normalize_recipients(request.cc, "cc")?;
    request.bcc = normalize_recipients(request.bcc, "bcc")?;
    request.subject = request.subject.trim().to_string();
    request.body = request.body.replace("\r\n", "\n").replace('\r', "\n");
    request.attachment_path = request
        .attachment_path
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty());
    if request.subject.is_empty()
        || request.subject.chars().count() > MAX_SUBJECT_CHARS
        || request.body.trim().is_empty()
        || request.body.chars().count() > MAX_BODY_CHARS
        || request.subject.contains('\0')
        || request.body.contains('\0')
        || request.attachment_path.as_deref().is_some_and(|path| {
            path.chars().count() > MAX_ATTACHMENT_PATH_CHARS || path.contains('\0')
        })
    {
        return Err("send_system_email content is outside the bounded contract.".to_string());
    }
    Ok(TaskToolValidation {
        arguments: serde_json::to_value(request).map_err(|error| error.to_string())?,
        potentially_effectful: true,
    })
}

fn normalize_recipients(value: Option<String>, field: &str) -> Result<Option<String>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.contains('\0') || value.chars().count() > MAX_RECIPIENT_TEXT_CHARS {
        return Err(format!(
            "send_system_email {field} recipients are outside the bounded contract."
        ));
    }
    let recipients = value
        .split(',')
        .map(str::trim)
        .filter(|recipient| !recipient.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if recipients.iter().any(|recipient| {
        let Some((local, domain)) = recipient.split_once('@') else {
            return true;
        };
        local.is_empty()
            || domain.is_empty()
            || domain.contains('@')
            || recipient.chars().any(char::is_whitespace)
    }) {
        return Err(format!(
            "send_system_email {field} must contain comma-separated email addresses."
        ));
    }
    Ok((!recipients.is_empty()).then(|| recipients.join(",")))
}

fn execute_registration<'a>(
    context: TaskToolExecutionContext<'a>,
    arguments: Value,
) -> TaskToolFuture<'a> {
    Box::pin(async move {
        let mut request =
            serde_json::from_value::<SendSystemEmailRequest>(arguments).map_err(|_| {
                "send_system_email arguments do not match the registered schema.".to_string()
            })?;
        let execution_id = context.execution_id.ok_or_else(|| {
            "Sending an email requires an active, explicitly approved Task.".to_string()
        })?;
        let task = require_agent_runtime_task(context.persistence, execution_id)?;
        let attachment = request
            .attachment_path
            .as_deref()
            .map(|path| verified_mail_attachment(context.persistence, &task.project_id, path))
            .transpose()?;
        request.attachment_path = attachment
            .as_ref()
            .map(|attachment| attachment.canonical_path.clone());
        let app = context
            .app
            .ok_or_else(|| "Sending an email requires the OOMU desktop app.".to_string())?;
        let result = app
            .send_mail(local_mail_send_request(&request))
            .await
            .map_err(|_| unverified_send_error())?;
        let response = response_from_receipt(&request, attachment.as_ref(), result)?;
        let receipt: Value =
            serde_json::from_str(&response.message).map_err(|_| unverified_send_error())?;
        record_event(
            context.persistence,
            &task.task_run_id,
            "mail.sent",
            EvidenceClass::VerifiedPostcondition,
            json!({
                "sentMessageIdSha256":receipt["sentMessageIdSha256"],
                "subjectSha256":receipt["subjectSha256"],
                "bodySha256":receipt["bodySha256"],
                "to":receipt["to"],
                "attachmentName":receipt["attachmentName"],
                "attachmentSha256":receipt["attachmentSha256"],
                "exactMatchCount":1,
                "uniquenessVerified":true,
            }),
        )?;
        Ok(response)
    })
}

fn verified_mail_attachment(
    persistence: &crate::db::PersistenceEngine,
    project_id: &str,
    path: &str,
) -> Result<VerifiedMailAttachment, String> {
    let binding = crate::shield_gate::bind_approved_external_file_read(path)
        .map_err(|error| error.message)?;
    let canonical = Path::new(&binding.canonical_path);
    let project_root =
        crate::projects::path_scope::single_active_project_root(persistence, project_id)?;
    if !canonical.starts_with(&project_root) {
        return Err(
            "The email attachment must belong to this Task's approved Project.".to_string(),
        );
    }
    let metadata = fs::metadata(canonical)
        .map_err(|_| "The email attachment could not be inspected safely.".to_string())?;
    if metadata.len() == 0 || metadata.len() > MAX_ATTACHMENT_BYTES {
        return Err("The email attachment is empty or exceeds the 25 MB limit.".to_string());
    }
    let file_name = canonical
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "The email attachment name is invalid.".to_string())?
        .to_string();
    Ok(VerifiedMailAttachment {
        canonical_path: binding.canonical_path.clone(),
        file_name,
        sha256: crate::foundation::digest::sha256_file_hex(canonical)
            .map_err(|error| error.to_string())?,
        byte_length: metadata.len(),
    })
}

fn response_from_receipt(
    request: &SendSystemEmailRequest,
    attachment: Option<&VerifiedMailAttachment>,
    result: LocalMailReceipt,
) -> Result<ExecuteCommandResponse, String> {
    let receipt = result.structured_content.as_ref();
    let expected_body_sha256 = crate::foundation::digest::sha256_hex(request.body.as_bytes());
    let expected_to = request_recipients(Some(&request.to));
    let expected_cc = request_recipients(request.cc.as_deref());
    let expected_bcc = request_recipients(request.bcc.as_deref());
    let sent_message_id = receipt
        .and_then(|value| value.get("sentMessageId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let verified = !result.is_error
        && receipt
            .and_then(|value| value.get("success"))
            .and_then(Value::as_bool)
            == Some(true)
        && receipt
            .and_then(|value| value.get("sent"))
            .and_then(Value::as_bool)
            == Some(true)
        && receipt
            .and_then(|value| value.get("verified"))
            .and_then(Value::as_bool)
            == Some(true)
        && receipt
            .and_then(|value| value.get("subject"))
            .and_then(Value::as_str)
            == Some(request.subject.as_str())
        && receipt
            .and_then(|value| value.get("bodySha256"))
            .and_then(Value::as_str)
            == Some(expected_body_sha256.as_str())
        && receipt_recipients(receipt, "to").as_ref() == Some(&expected_to)
        && receipt_recipients(receipt, "cc").as_ref() == Some(&expected_cc)
        && receipt_recipients(receipt, "bcc").as_ref() == Some(&expected_bcc)
        && receipt
            .and_then(|value| value.get("exactMatchCount"))
            .and_then(Value::as_u64)
            == Some(1)
        && receipt_attachment_matches(receipt, attachment)
        && receipt
            .and_then(|value| value.get("uniquenessVerified"))
            .and_then(Value::as_bool)
            == Some(true)
        && sent_message_id.is_some();
    if !verified {
        return Err(send_error_from_result(&result));
    }
    let sent_message_id = sent_message_id.expect("verified sent receipt has an id");
    let sent_message_id_sha256 = crate::foundation::digest::sha256_hex(sent_message_id.as_bytes());
    let subject_sha256 = crate::foundation::digest::sha256_hex(request.subject.as_bytes());
    let reused_existing = receipt
        .and_then(|value| value.get("reusedExisting"))
        .and_then(Value::as_bool)
        == Some(true);
    if let Some(attachment) = attachment {
        let final_sha256 =
            crate::foundation::digest::sha256_file_hex(Path::new(&attachment.canonical_path))
                .map_err(|error| error.to_string())?;
        if final_sha256 != attachment.sha256 {
            return Err("The email attachment changed while Mail was sending it.".to_string());
        }
    }
    Ok(ExecuteCommandResponse {
        operation: OPERATION.to_string(),
        status: CommandStatus::Completed,
        message: json!({
            "sentMessageIdSha256":sent_message_id_sha256,
            "subjectSha256":subject_sha256,
            "bodySha256":expected_body_sha256,
            "to":expected_to,
            "cc":expected_cc,
            "bcc":expected_bcc,
            "attachmentName":attachment.map(|value| value.file_name.as_str()),
            "attachmentSha256":attachment.map(|value| value.sha256.as_str()),
            "attachmentBytes":attachment.map(|value| value.byte_length),
            "sent":true,
            "verified":true,
            "exactMatchCount":1,
            "uniquenessVerified":true,
            "reusedExisting":reused_existing,
        }).to_string(),
        metrics: None,
        claims: vec![format!(
            "CLAIM mail_sent=true exact_match_count=1 unique=true reused_existing={reused_existing} sent_message_id_sha256={sent_message_id_sha256} subject_sha256={subject_sha256} body_sha256={expected_body_sha256} attachment_sha256={}",
            attachment.map(|value| value.sha256.as_str()).unwrap_or("none")
        )],
        verified: true,
        model_used: None,
    })
}

fn receipt_attachment_matches(
    receipt: Option<&Value>,
    attachment: Option<&VerifiedMailAttachment>,
) -> bool {
    match attachment {
        Some(expected) => {
            receipt
                .and_then(|value| value.get("attachmentName"))
                .and_then(Value::as_str)
                == Some(expected.file_name.as_str())
                && receipt
                    .and_then(|value| value.get("attachmentSha256"))
                    .and_then(Value::as_str)
                    == Some(expected.sha256.as_str())
                && receipt
                    .and_then(|value| value.get("attachmentBytes"))
                    .and_then(Value::as_u64)
                    == Some(expected.byte_length)
                && receipt
                    .and_then(|value| value.get("attachmentVerified"))
                    .and_then(Value::as_bool)
                    == Some(true)
        }
        None => receipt
            .and_then(|value| value.get("attachmentName"))
            .is_none_or(Value::is_null),
    }
}

fn send_error_from_result(result: &LocalMailReceipt) -> String {
    let receipt = result.structured_content.as_ref();
    let reported = receipt
        .and_then(|value| value.get("code"))
        .and_then(Value::as_str);
    let (code, message) = match reported {
        Some("mail_automation_permission_required") => (
            "mail_automation_permission_required",
            "macOS has not allowed OOMU to control Mail. No email was sent.",
        ),
        Some("mail_automation_timeout") => (
            "mail_automation_timeout",
            "Mail did not confirm Automation access in time. No email was sent.",
        ),
        Some("mail_automation_unavailable") => (
            "mail_automation_unavailable",
            "Mail Automation is unavailable right now. No email was sent.",
        ),
        Some("mail_attachment_unavailable") => (
            "mail_attachment_unavailable",
            "The report attachment is no longer available. No email was sent.",
        ),
        Some("mail_send_duplicate_detected") => (
            "mail_send_duplicate_detected",
            "Mail contains multiple matching sent messages. Review Sent Mail before continuing.",
        ),
        _ => (
            "mail_send_result_unverified",
            "Mail did not return a verifiable send result. Review Sent Mail before retrying.",
        ),
    };
    let mut context = Map::new();
    context.insert(
        "nextOperation".to_string(),
        Value::String(OPERATION.to_string()),
    );
    if let Some(phase) = receipt
        .and_then(|value| value.get("failurePhase"))
        .and_then(Value::as_str)
        .filter(|value| matches!(*value, "preflight" | "existing_lookup" | "postcondition"))
    {
        context.insert("failurePhase".to_string(), Value::String(phase.to_string()));
    }
    match receipt
        .and_then(|value| value.get("changedState"))
        .and_then(Value::as_str)
    {
        Some("none") => {
            context.insert("changedState".to_string(), Value::Bool(false));
        }
        Some("external_changes") => {
            context.insert(
                "changedState".to_string(),
                Value::String("external_changes".to_string()),
            );
        }
        _ => {}
    }
    json!({"taskToolError":{"code":code,"message":message,"context":context}}).to_string()
}

fn unverified_send_error() -> String {
    json!({"taskToolError":{
        "code":"mail_send_result_unverified",
        "message":"The trusted Mail bridge did not return a verifiable send result. Review Sent Mail before retrying.",
        "context":{}
    }}).to_string()
}

fn request_recipients(value: Option<&str>) -> Vec<String> {
    value
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|recipient| !recipient.is_empty())
        .map(str::to_string)
        .collect()
}

fn receipt_recipients(receipt: Option<&Value>, field: &str) -> Option<Vec<String>> {
    receipt?
        .get(field)?
        .as_array()?
        .iter()
        .map(|value| value.as_str().map(str::to_string))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(structured: Value, is_error: bool) -> LocalMailReceipt {
        LocalMailReceipt {
            structured_content: Some(structured),
            is_error,
        }
    }

    #[test]
    fn send_tool_is_distinct_bounded_and_explicitly_approved() {
        let _ = register_task_tool();
        assert_eq!(
            crate::tools::task_tool_runtime::approval_tier(OPERATION),
            Some(TaskToolApprovalTier::Explicit)
        );
        assert_eq!(
            crate::tools::task_tool_runtime::risk_tier(OPERATION).unwrap(),
            TaskToolRiskTier::SystemExec
        );
        let schema = crate::tools::task_tool_runtime::schema(OPERATION).unwrap();
        assert_eq!(schema["required"], json!(["to", "subject", "body"]));
        assert_eq!(schema["properties"]["attachmentPath"]["type"], "string");
        assert_eq!(schema["additionalProperties"], false);
        let validated = validate_registration(json!({
            "to":" owner@example.com ",
            "subject":" Exact subject ",
            "body":"Exact body"
        }))
        .unwrap();
        assert_eq!(validated.arguments["to"], "owner@example.com");
        assert!(validated.potentially_effectful);
        assert!(validate_registration(json!({
            "to":"owner@example.com",
            "subject":"Exact subject",
            "body":"Exact body",
            "draft":true
        }))
        .is_err());
    }

    #[test]
    fn only_unique_matching_sent_receipt_is_verified() {
        let request = SendSystemEmailRequest {
            to: "owner@example.com".to_string(),
            cc: None,
            bcc: None,
            subject: "OOMU Test — Supplier Exception".to_string(),
            body: "Report: supplier_exception_2026-07-21_10-30.md".to_string(),
            attachment_path: None,
        };
        let body_sha = crate::foundation::digest::sha256_hex(request.body.as_bytes());
        let response = response_from_receipt(
            &request,
            None,
            result(
                json!({
                    "success":true,
                    "sent":true,
                    "verified":true,
                    "sentMessageId":"sent-42",
                    "to":["owner@example.com"],
                    "cc":[],
                    "bcc":[],
                    "subject":request.subject,
                    "bodySha256":body_sha,
                    "exactMatchCount":1,
                    "uniquenessVerified":true,
                    "reusedExisting":false
                }),
                false,
            ),
        )
        .unwrap();
        assert!(response.verified);
        assert!(response.message.contains("\"sent\":true"));
        assert!(!response.message.contains("sent-42"));

        let error = response_from_receipt(
            &request,
            None,
            result(
                json!({
                    "success":true,
                    "sent":true,
                    "verified":true,
                    "sentMessageId":"sent-42",
                    "to":["owner@example.com"],
                    "cc":[],
                    "bcc":[],
                    "subject":request.subject,
                    "bodySha256":body_sha,
                    "exactMatchCount":2,
                    "uniquenessVerified":false
                }),
                false,
            ),
        )
        .unwrap_err();
        assert!(error.contains("mail_send_result_unverified"));
    }

    #[test]
    fn sent_receipt_must_verify_the_exact_attachment_identity() {
        let root = std::env::temp_dir().join(format!(
            "oomu-mail-attachment-{}",
            crate::p0_contracts::TaskId::new()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("report.md");
        std::fs::write(&path, b"verified report").unwrap();
        let attachment = VerifiedMailAttachment {
            canonical_path: path.to_string_lossy().to_string(),
            file_name: "report.md".to_string(),
            sha256: crate::foundation::digest::sha256_file_hex(&path).unwrap(),
            byte_length: 15,
        };
        let request = SendSystemEmailRequest {
            to: "owner@example.com".to_string(),
            cc: None,
            bcc: None,
            subject: "Verified report".to_string(),
            body: "The verified report is attached.".to_string(),
            attachment_path: Some(attachment.canonical_path.clone()),
        };
        let body_sha = crate::foundation::digest::sha256_hex(request.body.as_bytes());
        let response = response_from_receipt(
            &request,
            Some(&attachment),
            result(
                json!({
                    "success":true,"sent":true,"verified":true,
                    "sentMessageId":"sent-with-attachment","to":["owner@example.com"],
                    "cc":[],"bcc":[],"subject":request.subject,"bodySha256":body_sha,
                    "attachmentName":"report.md","attachmentSha256":attachment.sha256,
                    "attachmentBytes":attachment.byte_length,"attachmentVerified":true,"exactMatchCount":1,
                    "uniquenessVerified":true,"reusedExisting":false
                }),
                false,
            ),
        )
        .unwrap();
        assert!(response
            .message
            .contains("\"attachmentName\":\"report.md\""));

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn changed_state_is_truthful_for_send_failures() {
        let clean = send_error_from_result(&result(
            json!({
                "code":"mail_automation_permission_required",
                "failurePhase":"preflight",
                "changedState":"none"
            }),
            true,
        ));
        let normalized = crate::tools::task_tool_runtime::normalize_agent_error(OPERATION, &clean);
        let parsed = crate::tools::task_tool_runtime::parse_agent_error(&normalized).unwrap();
        assert!(parsed.changed_state_verified);
        assert_eq!(parsed.context["nextOperation"], OPERATION);
        assert_eq!(
            parsed.changed_state,
            crate::tools::task_tool_runtime::TaskToolChangedState::None
        );

        let uncertain = send_error_from_result(&result(
            json!({
                "code":"mail_send_result_unverified",
                "failurePhase":"postcondition",
                "changedState":"unverified"
            }),
            true,
        ));
        let normalized =
            crate::tools::task_tool_runtime::normalize_agent_error(OPERATION, &uncertain);
        let parsed = crate::tools::task_tool_runtime::parse_agent_error(&normalized).unwrap();
        assert!(!parsed.changed_state_verified);
    }
}
