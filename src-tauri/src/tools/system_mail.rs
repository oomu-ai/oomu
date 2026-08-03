use crate::{
    native_app_ports::{
        LocalApplicationMailPort, LocalMailReceipt, MailDraftContent,
        MailDraftPostconditionRequest, MailDraftRequest,
    },
    shield_gate::{CommandStatus, ExecuteCommandResponse},
    tools::{
        task_runtime::require_agent_runtime_task,
        task_tool_runtime::{
            TaskToolApprovalTier, TaskToolExecutionContext, TaskToolFuture, TaskToolMetadata,
            TaskToolRegistration, TaskToolRiskTier, TaskToolValidation,
        },
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const OPERATION: &str = "draft_system_email";
const MAX_RECIPIENT_TEXT_CHARS: usize = 4_096;
const MAX_SUBJECT_CHARS: usize = 998;
const MAX_BODY_CHARS: usize = 20_000;
const MAX_MAIL_RECEIPT_BYTES: usize = 32 * 1024;
const MAIL_AUTOMATION_PERMISSION_REQUIRED: &str = "mail_automation_permission_required";
const MAIL_AUTOMATION_TIMEOUT: &str = "mail_automation_timeout";
const MAIL_AUTOMATION_UNAVAILABLE: &str = "mail_automation_unavailable";
const MAIL_DRAFT_CREATION_FAILED_CLEANLY: &str = "mail_draft_creation_failed_cleanly";
const MAIL_DRAFT_REVIEW_REQUIRED: &str = "mail_draft_review_required";
const MAIL_DRAFT_RESULT_UNVERIFIED: &str = "mail_draft_result_unverified";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MailFailureChangedState {
    VerifiedUnchanged,
    ExternalChanges,
    Unverified,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MailFailure {
    code: &'static str,
    message: &'static str,
    failure_phase: Option<String>,
    cleanup_verified: Option<bool>,
    residual_draft_possible: Option<bool>,
    changed_state: MailFailureChangedState,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DraftSystemEmailRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bcc: Option<String>,
    subject: String,
    body: String,
}

fn mail_draft_content(request: &DraftSystemEmailRequest) -> MailDraftContent {
    MailDraftContent {
        to: request.to.clone(),
        cc: request.cc.clone(),
        bcc: request.bcc.clone(),
        subject: request.subject.clone(),
        body: request.body.clone(),
    }
}

pub(crate) fn register_task_tool() -> Result<(), String> {
    crate::tools::task_tool_runtime::register(TaskToolRegistration {
        operation: OPERATION,
        validate: validate_registration,
        validate_resolved: validate_registration,
        resolve: crate::tools::task_tool_runtime::identity_resolver,
        execute: execute_registration,
        planner_context: None,
        schema: draft_system_email_schema,
        metadata: TaskToolMetadata {
            description: "Create and save a visible, unsent draft in macOS Mail. This tool never sends the message.",
            risk_tier: TaskToolRiskTier::FileWrite,
            approval_tier: TaskToolApprovalTier::Explicit,
            agent_error_code: "mail_draft_creation_failed",
            agent_error_boundary: "SystemMailDraft",
            execution_path: "After explicit Shield approval, the trusted built-in Mail bridge saved the unsent draft and verified its native readback receipt.",
        },
    })
}

fn draft_system_email_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "to": {
                "type": "string",
                "maxLength": MAX_RECIPIENT_TEXT_CHARS,
                "description": "Optional comma-separated To recipients."
            },
            "cc": {
                "type": "string",
                "maxLength": MAX_RECIPIENT_TEXT_CHARS,
                "description": "Optional comma-separated CC recipients."
            },
            "bcc": {
                "type": "string",
                "maxLength": MAX_RECIPIENT_TEXT_CHARS,
                "description": "Optional comma-separated BCC recipients."
            },
            "subject": {
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_SUBJECT_CHARS
            },
            "body": {
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_BODY_CHARS
            }
        },
        "required": ["subject", "body"],
        "additionalProperties": false
    })
}

pub(crate) fn validate_registration(arguments: Value) -> Result<TaskToolValidation, String> {
    let mut request =
        serde_json::from_value::<DraftSystemEmailRequest>(arguments).map_err(|_| {
            "draft_system_email arguments do not match the registered schema.".to_string()
        })?;
    request.subject = request.subject.trim().to_string();
    request.body = request.body.replace("\r\n", "\n").replace('\r', "\n");
    request.to = normalize_recipients(request.to, "to")?;
    request.cc = normalize_recipients(request.cc, "cc")?;
    request.bcc = normalize_recipients(request.bcc, "bcc")?;

    if request.subject.is_empty()
        || request.subject.chars().count() > MAX_SUBJECT_CHARS
        || request.body.trim().is_empty()
        || request.body.chars().count() > MAX_BODY_CHARS
        || request.subject.contains('\0')
        || request.body.contains('\0')
    {
        return Err("draft_system_email content is outside the bounded contract.".to_string());
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
            "draft_system_email {field} recipients are outside the bounded contract."
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
            "draft_system_email {field} must contain comma-separated email addresses."
        ));
    }
    Ok((!recipients.is_empty()).then(|| recipients.join(",")))
}

fn safe_mail_failure_code(value: Option<&Value>) -> Option<&'static str> {
    match value?.get("code")?.as_str()? {
        MAIL_AUTOMATION_PERMISSION_REQUIRED => Some(MAIL_AUTOMATION_PERMISSION_REQUIRED),
        MAIL_AUTOMATION_TIMEOUT => Some(MAIL_AUTOMATION_TIMEOUT),
        MAIL_AUTOMATION_UNAVAILABLE => Some(MAIL_AUTOMATION_UNAVAILABLE),
        MAIL_DRAFT_CREATION_FAILED_CLEANLY => Some(MAIL_DRAFT_CREATION_FAILED_CLEANLY),
        MAIL_DRAFT_REVIEW_REQUIRED => Some(MAIL_DRAFT_REVIEW_REQUIRED),
        MAIL_DRAFT_RESULT_UNVERIFIED => Some(MAIL_DRAFT_RESULT_UNVERIFIED),
        _ => None,
    }
}

fn safe_mail_failure_phase(value: Option<&Value>) -> Option<String> {
    match value?.get("failurePhase")?.as_str()? {
        phase @ ("preflight" | "existing_lookup" | "bootstrap" | "populate_verify" | "cleanup"
        | "postcondition") => Some(phase.to_string()),
        _ => None,
    }
}

fn mail_failure_from_result(result: &LocalMailReceipt) -> MailFailure {
    let receipt = result.structured_content.as_ref();
    let reported_code = safe_mail_failure_code(receipt);
    let failure_phase = safe_mail_failure_phase(receipt);
    let cleanup_verified = receipt
        .and_then(|value| value.get("cleanupVerified"))
        .and_then(Value::as_bool);
    let residual_draft_possible = receipt
        .and_then(|value| value.get("residualDraftPossible"))
        .and_then(Value::as_bool);

    if residual_draft_possible == Some(true) {
        return MailFailure {
            code: MAIL_DRAFT_REVIEW_REQUIRED,
            message: "Mail could not verify that no unverified draft remains. Review Mail before continuing.",
            failure_phase,
            cleanup_verified,
            residual_draft_possible,
            changed_state: MailFailureChangedState::ExternalChanges,
        };
    }

    if result.is_error
        && failure_phase.as_deref() == Some("preflight")
        && matches!(
            reported_code,
            Some(
                MAIL_AUTOMATION_PERMISSION_REQUIRED
                    | MAIL_AUTOMATION_TIMEOUT
                    | MAIL_AUTOMATION_UNAVAILABLE
            )
        )
    {
        let code = reported_code.expect("preflight failure code was matched");
        let message = match code {
            MAIL_AUTOMATION_PERMISSION_REQUIRED => {
                "macOS has not allowed OOMU to control Mail. No Mail draft was created."
            }
            MAIL_AUTOMATION_TIMEOUT => {
                "Mail did not confirm Automation access in time. No Mail draft was created."
            }
            MAIL_AUTOMATION_UNAVAILABLE => {
                "Mail Automation is unavailable right now. No Mail draft was created."
            }
            _ => unreachable!("preflight failure code was allowlisted"),
        };
        return MailFailure {
            code,
            message,
            failure_phase,
            cleanup_verified,
            residual_draft_possible,
            changed_state: MailFailureChangedState::VerifiedUnchanged,
        };
    }

    if result.is_error && cleanup_verified == Some(true) && residual_draft_possible == Some(false) {
        return MailFailure {
            code: MAIL_DRAFT_CREATION_FAILED_CLEANLY,
            message:
                "Mail could not finish the draft, but verified that no unverified draft remains.",
            failure_phase,
            cleanup_verified,
            residual_draft_possible,
            changed_state: MailFailureChangedState::VerifiedUnchanged,
        };
    }

    MailFailure {
        code: MAIL_DRAFT_RESULT_UNVERIFIED,
        message: "Mail returned an unverified draft result. Review Mail before continuing.",
        failure_phase,
        cleanup_verified,
        residual_draft_possible,
        changed_state: MailFailureChangedState::Unverified,
    }
}

fn encoded_mail_failure(failure: &MailFailure) -> String {
    let mut context = serde_json::Map::new();
    if let Some(phase) = failure.failure_phase.as_ref() {
        context.insert("failurePhase".to_string(), Value::String(phase.clone()));
    }
    if let Some(value) = failure.cleanup_verified {
        context.insert("cleanupVerified".to_string(), Value::Bool(value));
    }
    if let Some(value) = failure.residual_draft_possible {
        context.insert("residualDraftPossible".to_string(), Value::Bool(value));
    }
    match failure.changed_state {
        MailFailureChangedState::VerifiedUnchanged => {
            context.insert("changedState".to_string(), Value::Bool(false));
        }
        MailFailureChangedState::ExternalChanges => {
            context.insert(
                "changedState".to_string(),
                Value::String("external_changes".to_string()),
            );
        }
        MailFailureChangedState::Unverified => {}
    }
    json!({
        "taskToolError": {
            "code": failure.code,
            "message": failure.message,
            "context": context,
        }
    })
    .to_string()
}

pub(crate) fn unverified_mail_result_error() -> String {
    encoded_mail_failure(&MailFailure {
        code: MAIL_DRAFT_RESULT_UNVERIFIED,
        message: "The trusted Mail bridge did not return a verifiable result. Review Mail before continuing.",
        failure_phase: None,
        cleanup_verified: None,
        residual_draft_possible: None,
        changed_state: MailFailureChangedState::Unverified,
    })
}

pub(crate) fn execute_registration<'a>(
    context: TaskToolExecutionContext<'a>,
    arguments: Value,
) -> TaskToolFuture<'a> {
    execute_registration_with_replay_protection(context, arguments, false)
}

pub(crate) fn execute_idempotent_registration<'a>(
    context: TaskToolExecutionContext<'a>,
    arguments: Value,
) -> TaskToolFuture<'a> {
    execute_registration_with_replay_protection(context, arguments, true)
}

fn execute_registration_with_replay_protection<'a>(
    context: TaskToolExecutionContext<'a>,
    arguments: Value,
    reuse_existing_matching: bool,
) -> TaskToolFuture<'a> {
    Box::pin(async move {
        let request =
            serde_json::from_value::<DraftSystemEmailRequest>(arguments).map_err(|_| {
                "draft_system_email arguments do not match the registered schema.".to_string()
            })?;
        let execution_id = context
            .execution_id
            .ok_or_else(|| "Creating a Mail draft requires an active approved Task.".to_string())?;
        require_agent_runtime_task(context.persistence, execution_id)?;
        let app = context
            .app
            .ok_or_else(|| "Creating a Mail draft requires the OOMU desktop app.".to_string())?;
        let result = app
            .create_mail_draft(MailDraftRequest {
                content: mail_draft_content(&request),
                reuse_existing_matching,
            })
            .await
            .map_err(|_| unverified_mail_result_error())?;
        response_from_mail_receipt(&request, result)
    })
}

pub(crate) async fn verify_exact_draft_postcondition(
    app: &tauri::AppHandle,
    arguments: Value,
    receipt_message: &str,
) -> Result<Value, String> {
    let validated = validate_registration(arguments)?;
    let request = serde_json::from_value::<DraftSystemEmailRequest>(validated.arguments.clone())
        .map_err(|_| {
            "draft_system_email arguments do not match the registered schema.".to_string()
        })?;
    let expected_draft_id = validated_persisted_mail_receipt(&request, receipt_message)?;
    let result = app
        .verify_mail_draft(MailDraftPostconditionRequest {
            content: mail_draft_content(&request),
        })
        .await
        .map_err(|_| unverified_mail_result_error())?;
    let postcondition_only = result
        .structured_content
        .as_ref()
        .and_then(|value| value.get("postconditionOnly"))
        .and_then(Value::as_bool)
        == Some(true);
    let response = response_from_mail_receipt(&request, result)?;
    if !postcondition_only || !response.verified {
        return Err(
            "Mail could not prove that exactly one matching unsent draft still exists.".to_string(),
        );
    }
    let final_receipt = serde_json::from_str::<Value>(&response.message)
        .map_err(|_| "Mail returned an invalid final postcondition receipt.".to_string())?;
    let final_draft_id = final_receipt
        .get("draftId")
        .and_then(Value::as_str)
        .ok_or_else(|| "Mail's final postcondition receipt has no draft identity.".to_string())?;
    if final_draft_id != expected_draft_id {
        return Err(
            "Mail's only matching unsent draft is not the draft recorded by this Task.".to_string(),
        );
    }
    Ok(json!({
        "verified": true,
        "exists": true,
        "sent": false,
        "exactMatchCount": 1,
        "uniquenessVerified": true,
        "draftIdSha256": crate::foundation::digest::sha256_hex(final_draft_id.as_bytes()),
        "subjectSha256": crate::foundation::digest::sha256_hex(request.subject.as_bytes()),
        "bodySha256": crate::foundation::digest::sha256_hex(request.body.as_bytes())
    }))
}

fn validated_persisted_mail_receipt(
    request: &DraftSystemEmailRequest,
    receipt_message: &str,
) -> Result<String, String> {
    if receipt_message.len() > MAX_MAIL_RECEIPT_BYTES {
        return Err("Mail receipt is too large to verify safely.".to_string());
    }
    let receipt = serde_json::from_str::<Value>(receipt_message)
        .map_err(|_| "Mail receipt is invalid.".to_string())?;
    let draft_id = receipt
        .get("draftId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Mail receipt has no draft identity.".to_string())?;
    let expected_body_sha256 = crate::foundation::digest::sha256_hex(request.body.as_bytes());
    let matches = receipt.get("subject").and_then(Value::as_str) == Some(request.subject.as_str())
        && receipt.get("bodySha256").and_then(Value::as_str) == Some(expected_body_sha256.as_str())
        && receipt_recipients(Some(&receipt), "to").as_ref()
            == Some(&request_recipients(&request.to))
        && receipt_recipients(Some(&receipt), "cc").as_ref()
            == Some(&request_recipients(&request.cc))
        && receipt_recipients(Some(&receipt), "bcc").as_ref()
            == Some(&request_recipients(&request.bcc))
        && receipt.get("draftState").and_then(Value::as_str) == Some("outgoing_message")
        && receipt.get("saved").and_then(Value::as_bool) == Some(true)
        && receipt.get("verified").and_then(Value::as_bool) == Some(true)
        && receipt.get("sent").and_then(Value::as_bool) == Some(false)
        && receipt.get("exactMatchCount").and_then(Value::as_u64) == Some(1)
        && receipt.get("uniquenessVerified").and_then(Value::as_bool) == Some(true);
    if !matches {
        return Err("Mail receipt does not match the verified unsent draft.".to_string());
    }
    Ok(draft_id.to_string())
}

fn response_from_mail_receipt(
    request: &DraftSystemEmailRequest,
    result: LocalMailReceipt,
) -> Result<ExecuteCommandResponse, String> {
    let receipt = result.structured_content.as_ref();
    let expected_body_sha256 = crate::foundation::digest::sha256_hex(request.body.as_bytes());
    let expected_to = request_recipients(&request.to);
    let expected_cc = request_recipients(&request.cc);
    let expected_bcc = request_recipients(&request.bcc);
    let draft_id = receipt
        .and_then(|value| value.get("draftId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let receipt_subject = receipt
        .and_then(|value| value.get("subject"))
        .and_then(Value::as_str);
    let receipt_is_verified = !result.is_error
        && receipt
            .and_then(|value| value.get("success"))
            .and_then(Value::as_bool)
            == Some(true)
        && receipt
            .and_then(|value| value.get("saved"))
            .and_then(Value::as_bool)
            == Some(true)
        && receipt
            .and_then(|value| value.get("verified"))
            .and_then(Value::as_bool)
            == Some(true)
        && draft_id.is_some()
        && receipt_subject == Some(request.subject.as_str())
        && receipt
            .and_then(|value| value.get("bodySha256"))
            .and_then(Value::as_str)
            == Some(expected_body_sha256.as_str())
        && receipt_recipients(receipt, "to").as_ref() == Some(&expected_to)
        && receipt_recipients(receipt, "cc").as_ref() == Some(&expected_cc)
        && receipt_recipients(receipt, "bcc").as_ref() == Some(&expected_bcc)
        && receipt
            .and_then(|value| value.get("draftState"))
            .and_then(Value::as_str)
            == Some("outgoing_message")
        && receipt
            .and_then(|value| value.get("sent"))
            .and_then(Value::as_bool)
            == Some(false)
        && receipt
            .and_then(|value| value.get("exactMatchCount"))
            .and_then(Value::as_u64)
            == Some(1)
        && receipt
            .and_then(|value| value.get("uniquenessVerified"))
            .and_then(Value::as_bool)
            == Some(true);

    if !receipt_is_verified {
        let failure = mail_failure_from_result(&result);
        if crate::scenario_one_e2e_profile::enabled() {
            eprintln!(
                "OOMU_SCENARIO_ONE_E2E_TRACE stage=mail_result status=failed code={} phase={} cleanup_verified={} residual_draft_possible={}",
                failure.code,
                failure.failure_phase.as_deref().unwrap_or("unknown"),
                failure
                    .cleanup_verified
                    .map(|value| if value { "true" } else { "false" })
                    .unwrap_or("unknown"),
                failure
                    .residual_draft_possible
                    .map(|value| if value { "true" } else { "false" })
                    .unwrap_or("unknown"),
            );
        }
        return Err(encoded_mail_failure(&failure));
    }

    let draft_id = draft_id.expect("verified receipt has a draft id");
    let draft_id_sha256 = crate::foundation::digest::sha256_hex(draft_id.as_bytes());
    let subject_sha256 = crate::foundation::digest::sha256_hex(request.subject.as_bytes());
    let reused_existing = receipt
        .and_then(|value| value.get("reusedExisting"))
        .and_then(Value::as_bool)
        == Some(true);
    Ok(ExecuteCommandResponse {
        operation: OPERATION.to_string(),
        status: CommandStatus::Completed,
        message: json!({
            "draftId": draft_id,
            "subject": request.subject,
            "bodySha256": expected_body_sha256.clone(),
            "to": expected_to,
            "cc": expected_cc,
            "bcc": expected_bcc,
            "draftState": "outgoing_message",
            "saved": true,
            "verified": true,
            "sent": false,
            "exactMatchCount": 1,
            "uniquenessVerified": true,
            "reusedExisting": reused_existing
        })
        .to_string(),
        metrics: None,
        claims: vec![format!(
            "CLAIM mail_draft_saved=true sent=false reused_existing={reused_existing} draft_id_sha256={draft_id_sha256} subject_sha256={subject_sha256} body_sha256={expected_body_sha256}"
        )],
        verified: true,
        model_used: None,
    })
}

fn request_recipients(value: &Option<String>) -> Vec<String> {
    value
        .as_deref()
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

    fn request() -> DraftSystemEmailRequest {
        DraftSystemEmailRequest {
            to: Some("owner@example.com".to_string()),
            cc: None,
            bcc: None,
            subject: "Supplier Decision Review".to_string(),
            body: "The decision pack is ready for review.".to_string(),
        }
    }

    fn result(structured_content: Value, is_error: bool) -> LocalMailReceipt {
        LocalMailReceipt {
            structured_content: Some(structured_content),
            is_error,
        }
    }

    #[test]
    fn registered_mail_tool_has_bounded_explicit_approval_contract() {
        let _ = register_task_tool();
        assert!(crate::tools::task_tool_runtime::is_registered(OPERATION));
        assert_eq!(
            crate::tools::task_tool_runtime::risk_tier(OPERATION).unwrap(),
            TaskToolRiskTier::FileWrite
        );
        assert_eq!(
            crate::tools::task_tool_runtime::approval_tier(OPERATION),
            Some(TaskToolApprovalTier::Explicit)
        );
        let schema = crate::tools::task_tool_runtime::schema(OPERATION).unwrap();
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["required"], json!(["subject", "body"]));
        for field in ["to", "cc", "bcc", "subject", "body"] {
            assert_eq!(schema["properties"][field]["type"], "string");
        }

        let validated = validate_registration(json!({
            "to": " owner@example.com, reviewer@example.com ",
            "subject": " Supplier Decision Review ",
            "body": "Ready."
        }))
        .unwrap();
        assert_eq!(
            validated.arguments["to"],
            "owner@example.com,reviewer@example.com"
        );
        assert_eq!(validated.arguments["subject"], "Supplier Decision Review");
        assert!(validated.potentially_effectful);
        assert!(validate_registration(json!({
            "to": "not-an-address",
            "subject": "Review",
            "body": "Ready."
        }))
        .is_err());
        assert!(validate_registration(json!({
            "subject": "Review",
            "body": "Ready.",
            "send": true
        }))
        .is_err());
        let planned = crate::tools::task_tool_runtime::PlannedTaskToolRequest::new(
            OPERATION,
            validated.arguments,
        );
        let action = crate::tools::task_tool_runtime::requested_action(&planned);
        let approval = crate::shield_gate::build_shield_approval_request(&action)
            .expect("Mail draft has explicit Shield semantics");
        assert_eq!(approval.approval_tier, "explicit_confirmation");
        assert!(crate::shield_gate::authorize_action(action.clone()).is_err());
        assert!(crate::shield_gate::authorize_action_for_approved_plan(action).is_ok());
    }

    #[test]
    fn receipt_is_verified_only_for_matching_saved_native_draft() {
        let request = request();
        let valid = response_from_mail_receipt(
            &request,
            result(
                json!({
                    "success": true,
                    "saved": true,
                    "verified": true,
                    "draftId": "draft-42",
                    "subject": "Supplier Decision Review",
                    "bodySha256": crate::foundation::digest::sha256_hex(request.body.as_bytes()),
                    "to": ["owner@example.com"],
                    "cc": [],
                    "bcc": [],
                    "draftState": "outgoing_message",
                    "sent": false,
                    "exactMatchCount": 1,
                    "uniquenessVerified": true
                }),
                false,
            ),
        )
        .unwrap();
        assert!(matches!(valid.status, CommandStatus::Completed));
        assert!(valid.verified);
        assert!(valid.message.contains("\"sent\":false"));
        assert!(valid.message.contains("\"exactMatchCount\":1"));
        assert!(valid.message.contains("\"uniquenessVerified\":true"));
        assert!(valid.message.contains("\"reusedExisting\":false"));
        assert_eq!(
            validated_persisted_mail_receipt(&request, &valid.message).unwrap(),
            "draft-42"
        );
        let mut non_unique_receipt: Value = serde_json::from_str(&valid.message).unwrap();
        non_unique_receipt["exactMatchCount"] = json!(2);
        non_unique_receipt["uniquenessVerified"] = json!(false);
        assert!(
            validated_persisted_mail_receipt(&request, &non_unique_receipt.to_string()).is_err()
        );

        let reused = response_from_mail_receipt(
            &request,
            result(
                json!({
                    "success": true,
                    "saved": true,
                    "verified": true,
                    "reusedExisting": true,
                    "draftId": "draft-42",
                    "subject": "Supplier Decision Review",
                    "bodySha256": crate::foundation::digest::sha256_hex(request.body.as_bytes()),
                    "to": ["owner@example.com"],
                    "cc": [],
                    "bcc": [],
                    "draftState": "outgoing_message",
                    "sent": false,
                    "exactMatchCount": 1,
                    "uniquenessVerified": true
                }),
                false,
            ),
        )
        .unwrap();
        assert!(reused.verified);
        assert!(reused.message.contains("\"reusedExisting\":true"));
        assert!(reused.claims[0].contains("reused_existing=true"));

        for invalid in [
            result(json!({}), false),
            result(
                json!({
                    "success": true,
                    "saved": true,
                    "verified": false,
                    "draftId": "draft-42",
                    "subject": "Supplier Decision Review"
                }),
                false,
            ),
            result(
                json!({
                    "success": true,
                    "saved": true,
                    "verified": true,
                    "draftId": "",
                    "subject": "Supplier Decision Review"
                }),
                false,
            ),
            result(
                json!({
                    "success": true,
                    "saved": true,
                    "verified": true,
                    "draftId": "draft-42",
                    "subject": "Different subject"
                }),
                false,
            ),
            result(
                json!({
                    "success": true,
                    "saved": true,
                    "verified": true,
                    "draftId": "draft-42",
                    "subject": "Supplier Decision Review"
                }),
                true,
            ),
            result(
                json!({
                    "success": true,
                    "saved": true,
                    "verified": true,
                    "draftId": "draft-42",
                    "subject": "Supplier Decision Review",
                    "bodySha256": crate::foundation::digest::sha256_hex(request.body.as_bytes()),
                    "to": ["owner@example.com"],
                    "cc": [],
                    "bcc": [],
                    "draftState": "outgoing_message",
                    "sent": false,
                    "exactMatchCount": 2,
                    "uniquenessVerified": false
                }),
                false,
            ),
        ] {
            let rejected = response_from_mail_receipt(&request, invalid).unwrap_err();
            let envelope: Value = serde_json::from_str(&rejected).unwrap();
            assert_eq!(
                envelope["taskToolError"]["code"],
                MAIL_DRAFT_RESULT_UNVERIFIED
            );
            assert!(envelope["taskToolError"]["context"]
                .get("changedState")
                .is_none());
        }

        let residual = response_from_mail_receipt(
            &request,
            result(
                json!({
                    "code": MAIL_DRAFT_REVIEW_REQUIRED,
                    "failurePhase": "cleanup",
                    "status": "error",
                    "cleanupVerified": false,
                    "residualDraftPossible": true
                }),
                true,
            ),
        )
        .unwrap_err();
        let envelope: Value = serde_json::from_str(&residual).unwrap();
        assert_eq!(
            envelope["taskToolError"]["code"],
            MAIL_DRAFT_REVIEW_REQUIRED
        );
        assert_eq!(
            envelope["taskToolError"]["context"]["changedState"],
            "external_changes"
        );
    }

    #[test]
    fn typed_mail_preflight_failures_are_verified_unchanged_without_payload_leakage() {
        let request = request();
        for code in [
            MAIL_AUTOMATION_PERMISSION_REQUIRED,
            MAIL_AUTOMATION_TIMEOUT,
            MAIL_AUTOMATION_UNAVAILABLE,
        ] {
            let error = response_from_mail_receipt(
                &request,
                result(
                    json!({
                        "code": code,
                        "failurePhase": "preflight",
                    }),
                    true,
                ),
            )
            .unwrap_err();
            let envelope: Value = serde_json::from_str(&error).unwrap();
            assert_eq!(envelope["taskToolError"]["code"], code);
            assert_eq!(
                envelope["taskToolError"]["context"]["failurePhase"],
                "preflight"
            );
            assert_eq!(envelope["taskToolError"]["context"]["changedState"], false);
            assert!(!error.contains("owner@example.com"));
            assert!(!error.contains("Supplier Decision Review"));
            assert!(!error.contains("ready for review"));
        }
    }

    #[test]
    fn verified_cleanup_is_retry_safe_but_contradictory_or_unknown_evidence_fails_closed() {
        let request = request();
        let cleaned = response_from_mail_receipt(
            &request,
            result(
                json!({
                    "code": MAIL_DRAFT_CREATION_FAILED_CLEANLY,
                    "failurePhase": "cleanup",
                    "cleanupVerified": true,
                    "residualDraftPossible": false,
                }),
                true,
            ),
        )
        .unwrap_err();
        let cleaned: Value = serde_json::from_str(&cleaned).unwrap();
        assert_eq!(
            cleaned["taskToolError"]["code"],
            MAIL_DRAFT_CREATION_FAILED_CLEANLY
        );
        assert_eq!(cleaned["taskToolError"]["context"]["changedState"], false);

        for structured in [
            json!({
                "code": MAIL_DRAFT_CREATION_FAILED_CLEANLY,
                "failurePhase": "cleanup",
                "cleanupVerified": true,
            }),
            json!({
                "code": MAIL_AUTOMATION_PERMISSION_REQUIRED,
                "failurePhase": "bootstrap",
            }),
            json!({
                "code": "untrusted_mail_code",
                "failurePhase": "preflight",
            }),
        ] {
            let error = response_from_mail_receipt(&request, result(structured, true)).unwrap_err();
            let envelope: Value = serde_json::from_str(&error).unwrap();
            assert_eq!(
                envelope["taskToolError"]["code"],
                MAIL_DRAFT_RESULT_UNVERIFIED
            );
            assert!(envelope["taskToolError"]["context"]
                .get("changedState")
                .is_none());
        }
    }

    #[test]
    fn any_possible_residual_draft_requires_external_review() {
        let request = request();
        for structured in [
            json!({
                "code": MAIL_DRAFT_REVIEW_REQUIRED,
                "failurePhase": "existing_lookup",
                "residualDraftPossible": true,
            }),
            json!({
                "code": MAIL_AUTOMATION_PERMISSION_REQUIRED,
                "failurePhase": "preflight",
                "cleanupVerified": true,
                "residualDraftPossible": true,
            }),
        ] {
            let error = response_from_mail_receipt(&request, result(structured, true)).unwrap_err();
            let envelope: Value = serde_json::from_str(&error).unwrap();
            assert_eq!(
                envelope["taskToolError"]["code"],
                MAIL_DRAFT_REVIEW_REQUIRED
            );
            assert_eq!(
                envelope["taskToolError"]["context"]["changedState"],
                "external_changes"
            );
        }
    }
}
