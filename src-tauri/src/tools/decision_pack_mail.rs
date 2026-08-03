use crate::{
    db::PersistenceEngine,
    shield_gate::{CommandStatus, ExecuteCommandResponse},
    tools::{
        system_mail,
        task_runtime::require_agent_runtime_task,
        task_tool_runtime::{
            TaskToolApprovalTier, TaskToolExecutionContext, TaskToolFuture, TaskToolMetadata,
            TaskToolRegistration, TaskToolRiskTier, TaskToolValidation,
        },
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Seek, SeekFrom},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Component, Path},
};

const OPERATION: &str = "draft_decision_pack_email";
const SOURCE_OPERATION: &str = "create_decision_pack";
const RECEIPT_SCHEMA_VERSION: u32 = 1;
const MAX_ARGUMENT_BYTES: usize = 32 * 1024;
const MAX_RECEIPT_BYTES: usize = 64 * 1024;
const MAX_PATH_CHARS: usize = 4_096;
const MAX_RECOMMENDATION_CHARS: usize = 8_000;
const MAX_EMAIL_SUMMARY_CHARS: usize = 8_000;
const MAX_OUTPUT_FILE_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DraftDecisionPackEmailRequest {
    to: String,
    subject: String,
    expected_output_paths: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResolvedDraftDecisionPackEmailRequest {
    to: String,
    subject: String,
    expected_output_paths: Vec<String>,
    analysis_sha256: String,
    recommendation: String,
    email_summary: String,
    files: Vec<DecisionPackFileReceipt>,
    body: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DecisionPackReceipt {
    schema_version: u32,
    analysis_sha256: String,
    recommendation: String,
    email_summary: String,
    files: Vec<DecisionPackFileReceipt>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DecisionPackFileReceipt {
    kind: DecisionPackFileKind,
    path: String,
    sha256: String,
    byte_count: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum DecisionPackFileKind {
    Workbook,
    Presentation,
    Pdf,
    Sources,
}

impl DecisionPackFileKind {
    fn extension(self) -> &'static str {
        match self {
            Self::Workbook => "xlsx",
            Self::Presentation => "pptx",
            Self::Pdf => "pdf",
            Self::Sources => "md",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Workbook => "Workbook",
            Self::Presentation => "Presentation",
            Self::Pdf => "PDF",
            Self::Sources => "Sources",
        }
    }

    fn rank(self) -> u8 {
        match self {
            Self::Workbook => 0,
            Self::Presentation => 1,
            Self::Pdf => 2,
            Self::Sources => 3,
        }
    }
}

pub(crate) fn register_task_tool() -> Result<(), String> {
    crate::tools::task_tool_runtime::register(TaskToolRegistration {
        operation: OPERATION,
        validate: validate_registration,
        validate_resolved: validate_resolved_registration,
        resolve: resolve_registration,
        execute: execute_registration,
        planner_context: None,
        schema: draft_decision_pack_email_schema,
        metadata: TaskToolMetadata {
            description: "Create and verify one visible, unsent macOS Mail draft from a prior verified decision-pack receipt. This tool never sends the message.",
            risk_tier: TaskToolRiskTier::FileWrite,
            approval_tier: TaskToolApprovalTier::Explicit,
            agent_error_code: "decision_pack_mail_draft_failed",
            agent_error_boundary: "DecisionPackMailDraft",
            execution_path: "After separate explicit Shield approval, the decision-pack Mail step bound its body to the verified pack receipt and reused the trusted unsent Mail-draft runtime.",
        },
    })
}

fn draft_decision_pack_email_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "to": {"type": "string", "minLength": 3, "maxLength": 4096},
            "subject": {"type": "string", "minLength": 1, "maxLength": 998},
            "expectedOutputPaths": {
                "type": "array",
                "minItems": 4,
                "maxItems": 4,
                "uniqueItems": true,
                "items": {"type": "string", "minLength": 1, "maxLength": MAX_PATH_CHARS}
            }
        },
        "required": ["to", "subject", "expectedOutputPaths"],
        "additionalProperties": false
    })
}

fn validate_registration(arguments: Value) -> Result<TaskToolValidation, String> {
    ensure_json_size(&arguments, MAX_ARGUMENT_BYTES, "arguments")?;
    let request =
        serde_json::from_value::<DraftDecisionPackEmailRequest>(arguments).map_err(|_| {
            "draft_decision_pack_email arguments do not match the registered schema.".to_string()
        })?;
    let request = normalize_public_request(request)?;
    Ok(TaskToolValidation {
        arguments: serde_json::to_value(request).map_err(|error| error.to_string())?,
        potentially_effectful: true,
    })
}

fn validate_resolved_registration(arguments: Value) -> Result<TaskToolValidation, String> {
    ensure_json_size(&arguments, MAX_RECEIPT_BYTES, "resolved arguments")?;
    let mut request = serde_json::from_value::<ResolvedDraftDecisionPackEmailRequest>(arguments)
        .map_err(|_| {
            "draft_decision_pack_email resolved arguments are invalid runtime state.".to_string()
        })?;
    let normalized = normalize_public_request(DraftDecisionPackEmailRequest {
        to: request.to.clone(),
        subject: request.subject.clone(),
        expected_output_paths: request.expected_output_paths.clone(),
    })?;
    request.to = normalized.to;
    request.subject = normalized.subject;
    request.expected_output_paths = normalized.expected_output_paths;

    let mut receipt = DecisionPackReceipt {
        schema_version: RECEIPT_SCHEMA_VERSION,
        analysis_sha256: request.analysis_sha256,
        recommendation: request.recommendation,
        email_summary: request.email_summary,
        files: request.files,
    };
    normalize_and_validate_receipt(&mut receipt)?;
    require_exact_path_set(&request.expected_output_paths, &receipt.files)?;
    let body = validated_mail_body(
        &request.to,
        &request.subject,
        &receipt.email_summary,
        &receipt.recommendation,
        &receipt.files,
    )?;
    if request.body != body {
        return Err("decision_pack_mail_body_not_receipt_bound".to_string());
    }

    let resolved = ResolvedDraftDecisionPackEmailRequest {
        to: request.to,
        subject: request.subject,
        expected_output_paths: request.expected_output_paths,
        analysis_sha256: receipt.analysis_sha256,
        recommendation: receipt.recommendation,
        email_summary: receipt.email_summary,
        files: receipt.files,
        body,
    };
    Ok(TaskToolValidation {
        arguments: serde_json::to_value(resolved).map_err(|error| error.to_string())?,
        potentially_effectful: true,
    })
}

/// Matches the complete receipt-bound Mail payload used after the decision
/// pack step resolves. This deliberately validates all eight internal fields;
/// callers must not authorize the later mutation from the three-field planner
/// request alone.
#[cfg(any(debug_assertions, test))]
pub(crate) fn resolved_preview_matches_exact_contract(
    value: &Value,
    expected_recipient: &str,
    expected_subject: &str,
    expected_output_paths: &[&str],
) -> bool {
    let Ok(validated) = validate_resolved_registration(value.clone()) else {
        return false;
    };
    let Ok(request) =
        serde_json::from_value::<ResolvedDraftDecisionPackEmailRequest>(validated.arguments)
    else {
        return false;
    };
    let actual_paths = request
        .expected_output_paths
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let expected_paths = expected_output_paths
        .iter()
        .copied()
        .collect::<HashSet<_>>();
    request.to.eq_ignore_ascii_case(expected_recipient.trim())
        && request.subject == expected_subject
        && actual_paths.len() == expected_output_paths.len()
        && actual_paths == expected_paths
}

fn normalize_public_request(
    mut request: DraftDecisionPackEmailRequest,
) -> Result<DraftDecisionPackEmailRequest, String> {
    let normalized_mail = system_mail::validate_registration(json!({
        "to": request.to,
        "subject": request.subject,
        "body": "Decision-pack receipt pending."
    }))?;
    request.to = required_string(&normalized_mail.arguments, "to")?;
    request.subject = required_string(&normalized_mail.arguments, "subject")?;
    if request.to.contains(',') || request.subject.chars().any(char::is_control) {
        return Err(
            "draft_decision_pack_email requires one recipient and a single-line subject."
                .to_string(),
        );
    }
    validate_expected_paths(&request.expected_output_paths)?;
    request
        .expected_output_paths
        .sort_by_key(|path| extension_rank(path).unwrap_or(u8::MAX));
    Ok(request)
}

fn validate_expected_paths(paths: &[String]) -> Result<(), String> {
    if paths.len() != 4 {
        return Err("draft_decision_pack_email requires exactly four output paths.".to_string());
    }
    let mut distinct_paths = HashSet::new();
    let mut distinct_extensions = HashSet::new();
    for value in paths {
        validate_absolute_output_path(value)?;
        let extension = Path::new(value)
            .extension()
            .and_then(|value| value.to_str())
            .ok_or_else(|| "decision_pack_mail_output_extension_invalid".to_string())?;
        if !matches!(extension, "xlsx" | "pptx" | "pdf" | "md")
            || !distinct_paths.insert(value.as_str())
            || !distinct_extensions.insert(extension)
        {
            return Err(
                "draft_decision_pack_email output paths must be distinct and contain one .xlsx, .pptx, .pdf, and .md file."
                    .to_string(),
            );
        }
    }
    Ok(())
}

fn validate_absolute_output_path(value: &str) -> Result<(), String> {
    let path = Path::new(value);
    if value.chars().any(char::is_control)
        || value.chars().count() > MAX_PATH_CHARS
        || !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(
            "draft_decision_pack_email output paths must be bounded absolute paths without traversal."
                .to_string(),
        );
    }
    Ok(())
}

fn resolve_registration(
    _persistence: &PersistenceEngine,
    _execution_id: Option<&str>,
    arguments: Value,
    outputs: &[ExecuteCommandResponse],
) -> Result<Value, String> {
    resolve_from_prior_outputs(arguments, outputs)
}

pub(crate) fn resolve_and_verify_postcondition(
    persistence: &PersistenceEngine,
    execution_id: Option<&str>,
    planned_mail_arguments: Value,
    outputs: &[ExecuteCommandResponse],
) -> Result<Value, String> {
    let execution_id = execution_id.ok_or_else(|| {
        "Decision-pack postcondition verification requires an active approved Task.".to_string()
    })?;
    require_agent_runtime_task(persistence, execution_id)?;
    resolved_mail_postcondition_arguments(planned_mail_arguments, outputs)
}

fn resolved_mail_postcondition_arguments(
    planned_mail_arguments: Value,
    outputs: &[ExecuteCommandResponse],
) -> Result<Value, String> {
    let resolved = resolve_from_prior_outputs(planned_mail_arguments, outputs)?;
    let validated = validate_resolved_registration(resolved)?;
    let request =
        serde_json::from_value::<ResolvedDraftDecisionPackEmailRequest>(validated.arguments)
            .map_err(|_| {
                "draft_decision_pack_email resolved arguments are invalid runtime state."
                    .to_string()
            })?;
    verify_current_output_files(&request.files)?;
    system_mail::validate_registration(json!({
        "to": request.to,
        "subject": request.subject,
        "body": request.body
    }))
    .map(|validation| validation.arguments)
}

fn resolve_from_prior_outputs(
    arguments: Value,
    outputs: &[ExecuteCommandResponse],
) -> Result<Value, String> {
    let request =
        serde_json::from_value::<DraftDecisionPackEmailRequest>(arguments).map_err(|_| {
            "draft_decision_pack_email arguments do not match the registered schema.".to_string()
        })?;
    let request = normalize_public_request(request)?;
    let output = outputs
        .iter()
        .rev()
        .find(|output| output.operation == SOURCE_OPERATION)
        .ok_or_else(|| "decision_pack_mail_prior_pack_missing".to_string())?;
    if !output.verified || !matches!(&output.status, CommandStatus::Completed) {
        return Err("decision_pack_mail_prior_pack_not_verified".to_string());
    }
    if output.message.len() > MAX_RECEIPT_BYTES {
        return Err("decision_pack_mail_prior_pack_receipt_too_large".to_string());
    }
    let mut receipt = serde_json::from_str::<DecisionPackReceipt>(&output.message)
        .map_err(|_| "decision_pack_mail_prior_pack_receipt_invalid".to_string())?;
    normalize_and_validate_receipt(&mut receipt)?;
    require_exact_path_set(&request.expected_output_paths, &receipt.files)?;
    let body = validated_mail_body(
        &request.to,
        &request.subject,
        &receipt.email_summary,
        &receipt.recommendation,
        &receipt.files,
    )?;
    serde_json::to_value(ResolvedDraftDecisionPackEmailRequest {
        to: request.to,
        subject: request.subject,
        expected_output_paths: request.expected_output_paths,
        analysis_sha256: receipt.analysis_sha256,
        recommendation: receipt.recommendation,
        email_summary: receipt.email_summary,
        files: receipt.files,
        body,
    })
    .map_err(|error| error.to_string())
}

fn normalize_and_validate_receipt(receipt: &mut DecisionPackReceipt) -> Result<(), String> {
    if receipt.schema_version != RECEIPT_SCHEMA_VERSION
        || !is_sha256(&receipt.analysis_sha256)
        || receipt.files.len() != 4
    {
        return Err("decision_pack_mail_prior_pack_receipt_invalid".to_string());
    }
    receipt.recommendation = bounded_receipt_text(
        &receipt.recommendation,
        MAX_RECOMMENDATION_CHARS,
        "recommendation",
    )?;
    receipt.email_summary = bounded_receipt_text(
        &receipt.email_summary,
        MAX_EMAIL_SUMMARY_CHARS,
        "email_summary",
    )?;

    let mut kinds = HashSet::new();
    let mut paths = HashSet::new();
    for file in &receipt.files {
        validate_absolute_output_path(&file.path)?;
        if file.byte_count == 0
            || file.byte_count > MAX_OUTPUT_FILE_BYTES
            || !is_sha256(&file.sha256)
            || file.path.rsplit_once('.').map(|(_, extension)| extension)
                != Some(file.kind.extension())
            || !kinds.insert(file.kind)
            || !paths.insert(file.path.as_str())
        {
            return Err("decision_pack_mail_prior_pack_file_receipt_invalid".to_string());
        }
    }
    receipt.files.sort_by_key(|file| file.kind.rank());
    Ok(())
}

fn require_exact_path_set(
    expected_paths: &[String],
    files: &[DecisionPackFileReceipt],
) -> Result<(), String> {
    let expected = expected_paths
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let received = files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<HashSet<_>>();
    if expected != received {
        return Err("decision_pack_mail_output_paths_mismatch".to_string());
    }
    Ok(())
}

fn validated_mail_body(
    to: &str,
    subject: &str,
    email_summary: &str,
    recommendation: &str,
    files: &[DecisionPackFileReceipt],
) -> Result<String, String> {
    let file_list = files
        .iter()
        .map(|file| format!("- {}: {}", file.kind.label(), file.path))
        .collect::<Vec<_>>()
        .join("\n");
    let body = format!(
        "{email_summary}\n\nRecommendation:\n{recommendation}\n\nDecision pack files:\n{file_list}"
    );
    let validation = system_mail::validate_registration(json!({
        "to": to,
        "subject": subject,
        "body": body
    }))?;
    required_string(&validation.arguments, "body")
}

fn execute_registration<'a>(
    context: TaskToolExecutionContext<'a>,
    arguments: Value,
) -> TaskToolFuture<'a> {
    Box::pin(async move {
        let validated = validate_resolved_registration(arguments)?;
        let request =
            serde_json::from_value::<ResolvedDraftDecisionPackEmailRequest>(validated.arguments)
                .map_err(|_| {
                    "draft_decision_pack_email resolved arguments are invalid.".to_string()
                })?;
        verify_current_output_files(&request.files)?;
        let mail = system_mail::validate_registration(json!({
            "to": request.to,
            "subject": request.subject,
            "body": request.body
        }))?;
        let mail_context = TaskToolExecutionContext {
            persistence: context.persistence,
            identity: context.identity,
            app: context.app,
            execution_id: context.execution_id,
            plan_id: context.plan_id,
            objective: context.objective,
            session_id: context.session_id,
            model_route: context.model_route,
        };
        let mut response =
            system_mail::execute_idempotent_registration(mail_context, mail.arguments).await?;
        response.operation = OPERATION.to_string();
        Ok(response)
    })
}

fn verify_current_output_files(files: &[DecisionPackFileReceipt]) -> Result<(), String> {
    for receipt in files {
        let path = Path::new(&receipt.path);
        let path_metadata = fs::symlink_metadata(path).map_err(|_| {
            format!(
                "The {} decision-pack file no longer exists. No Mail draft was created.",
                receipt.kind.label()
            )
        })?;
        if path_metadata.file_type().is_symlink()
            || !path_metadata.is_file()
            || path_metadata.len() != receipt.byte_count
        {
            return Err(format!(
                "The {} decision-pack file changed after verification. No Mail draft was created.",
                receipt.kind.label()
            ));
        }
        let mut file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(path)
            .map_err(|_| {
                format!(
                    "The {} decision-pack file could not be reopened safely. No Mail draft was created.",
                    receipt.kind.label()
                )
            })?;
        let open_metadata = file.metadata().map_err(|_| {
            format!(
                "The {} decision-pack file could not be rechecked. No Mail draft was created.",
                receipt.kind.label()
            )
        })?;
        if open_metadata.dev() != path_metadata.dev()
            || open_metadata.ino() != path_metadata.ino()
            || open_metadata.len() != receipt.byte_count
        {
            return Err(format!(
                "The {} decision-pack path changed during verification. No Mail draft was created.",
                receipt.kind.label()
            ));
        }
        let expected_magic = match receipt.kind {
            DecisionPackFileKind::Workbook | DecisionPackFileKind::Presentation => b"PK".as_slice(),
            DecisionPackFileKind::Pdf => b"%PDF-".as_slice(),
            DecisionPackFileKind::Sources => b"# Approved local inputs".as_slice(),
        };
        let mut prefix = vec![0_u8; expected_magic.len()];
        file.read_exact(&mut prefix).map_err(|_| {
            format!(
                "The {} decision-pack file is truncated. No Mail draft was created.",
                receipt.kind.label()
            )
        })?;
        if prefix != expected_magic {
            return Err(format!(
                "The {} decision-pack file no longer has its verified format. No Mail draft was created.",
                receipt.kind.label()
            ));
        }
        file.seek(SeekFrom::Start(0))
            .map_err(|_| "A decision-pack file could not be rehashed.".to_string())?;
        let digest =
            crate::foundation::digest::sha256_reader_bounded(&mut file, MAX_OUTPUT_FILE_BYTES)
                .map_err(|_| "A decision-pack file could not be rehashed.".to_string())?
                .ok_or_else(|| "A decision-pack file exceeds the revalidation limit.".to_string())?
                .to_hex();
        let final_path_metadata = fs::symlink_metadata(path).map_err(|_| {
            "A decision-pack file disappeared during Mail-draft verification.".to_string()
        })?;
        if digest != receipt.sha256
            || final_path_metadata.file_type().is_symlink()
            || final_path_metadata.dev() != open_metadata.dev()
            || final_path_metadata.ino() != open_metadata.ino()
            || final_path_metadata.len() != receipt.byte_count
        {
            return Err(format!(
                "The {} decision-pack file no longer matches its verified receipt. No Mail draft was created.",
                receipt.kind.label()
            ));
        }
    }
    Ok(())
}

fn bounded_receipt_text(value: &str, maximum: usize, field: &str) -> Result<String, String> {
    let value = value.replace("\r\n", "\n").replace('\r', "\n");
    let value = value.trim();
    if value.is_empty()
        || value
            .chars()
            .any(|character| character.is_control() && character != '\n' && character != '\t')
        || value.chars().count() > maximum
    {
        return Err(format!("decision_pack_mail_prior_pack_{field}_invalid"));
    }
    Ok(value.to_string())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn required_string(value: &Value, field: &str) -> Result<String, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("decision_pack_mail_{field}_missing"))
}

fn extension_rank(path: &str) -> Option<u8> {
    match Path::new(path).extension()?.to_str()? {
        "xlsx" => Some(0),
        "pptx" => Some(1),
        "pdf" => Some(2),
        "md" => Some(3),
        _ => None,
    }
}

fn ensure_json_size(value: &Value, maximum: usize, field: &str) -> Result<(), String> {
    if serde_json::to_vec(value)
        .map_err(|error| error.to_string())?
        .len()
        > maximum
    {
        return Err(format!("draft_decision_pack_email {field} are too large."));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expected_paths() -> Vec<String> {
        vec![
            "/tmp/ship_test_01/supplier_decision.xlsx".to_string(),
            "/tmp/ship_test_01/supplier_decision.pptx".to_string(),
            "/tmp/ship_test_01/supplier_decision.pdf".to_string(),
            "/tmp/ship_test_01/sources.md".to_string(),
        ]
    }

    fn arguments() -> Value {
        json!({
            "to": "recipient@example.com",
            "subject": "Supplier Decision Review",
            "expectedOutputPaths": expected_paths()
        })
    }

    fn receipt(paths: &[String]) -> Value {
        json!({
            "schemaVersion": 1,
            "analysisSha256": "a".repeat(64),
            "recommendation": "Select the supplier with the strongest verified margin.",
            "emailSummary": "The supplier decision pack is ready for review.",
            "files": [
                {"kind":"workbook","path":paths[0],"sha256":"b".repeat(64),"byteCount":101},
                {"kind":"presentation","path":paths[1],"sha256":"c".repeat(64),"byteCount":102},
                {"kind":"pdf","path":paths[2],"sha256":"d".repeat(64),"byteCount":103},
                {"kind":"sources","path":paths[3],"sha256":"e".repeat(64),"byteCount":104}
            ]
        })
    }

    fn pack_output(message: Value) -> ExecuteCommandResponse {
        ExecuteCommandResponse {
            operation: SOURCE_OPERATION.to_string(),
            status: CommandStatus::Completed,
            message: message.to_string(),
            metrics: None,
            claims: vec!["CLAIM decision_pack_verified=true".to_string()],
            verified: true,
            model_used: None,
        }
    }

    #[test]
    fn mail_preflight_reopens_and_rehashes_every_output() {
        let root = std::env::temp_dir().join(format!(
            "oomu-decision-pack-mail-recheck-{}-{}",
            std::process::id(),
            crate::foundation::clock::unix_time_ms_i64()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let payloads = [
            (
                DecisionPackFileKind::Workbook,
                "decision.xlsx",
                b"PK workbook".as_slice(),
            ),
            (
                DecisionPackFileKind::Presentation,
                "decision.pptx",
                b"PK presentation".as_slice(),
            ),
            (
                DecisionPackFileKind::Pdf,
                "decision.pdf",
                b"%PDF-1.7".as_slice(),
            ),
            (
                DecisionPackFileKind::Sources,
                "sources.md",
                b"# Approved local inputs\n",
            ),
        ];
        let files = payloads
            .iter()
            .map(|(kind, name, bytes)| {
                let path = root.join(name);
                std::fs::write(&path, bytes).unwrap();
                DecisionPackFileReceipt {
                    kind: *kind,
                    path: path.to_string_lossy().to_string(),
                    sha256: crate::foundation::digest::sha256_hex(bytes),
                    byte_count: bytes.len() as u64,
                }
            })
            .collect::<Vec<_>>();
        verify_current_output_files(&files).unwrap();
        let planned_arguments = json!({
            "to": "recipient@example.com",
            "subject": "Supplier Decision Review",
            "expectedOutputPaths": files.iter().map(|file| file.path.clone()).collect::<Vec<_>>()
        });
        let pack_receipt = serde_json::to_value(DecisionPackReceipt {
            schema_version: RECEIPT_SCHEMA_VERSION,
            analysis_sha256: "a".repeat(64),
            recommendation: "Select the supplier with the strongest verified margin.".to_string(),
            email_summary: "The supplier decision pack is ready for review.".to_string(),
            files: files.clone(),
        })
        .unwrap();
        let mail_arguments =
            resolved_mail_postcondition_arguments(planned_arguments, &[pack_output(pack_receipt)])
                .unwrap();
        assert_eq!(mail_arguments["to"], "recipient@example.com");
        assert_eq!(mail_arguments["subject"], "Supplier Decision Review");
        assert!(mail_arguments["body"]
            .as_str()
            .is_some_and(|body| body.contains("Decision pack files:")));
        std::fs::write(&files[0].path, b"PK changed!").unwrap();
        let error = verify_current_output_files(&files).unwrap_err();
        assert!(error.contains("no longer matches its verified receipt"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resolver_injects_body_only_from_verified_matching_pack_receipt() {
        let paths = expected_paths();
        let resolved = resolve_from_prior_outputs(arguments(), &[pack_output(receipt(&paths))])
            .expect("verified receipt resolves");
        let request = serde_json::from_value::<ResolvedDraftDecisionPackEmailRequest>(resolved)
            .expect("internal request");
        assert!(request.body.contains("The supplier decision pack is ready"));
        assert!(request.body.contains("strongest verified margin"));
        for path in paths {
            assert!(request.body.contains(&path));
        }
        assert!(validate_resolved_registration(
            serde_json::to_value(request).expect("serialize internal request")
        )
        .is_ok());
    }

    #[test]
    fn resolver_rejects_unverified_create_decision_pack_output() {
        let paths = expected_paths();
        let mut output = pack_output(receipt(&paths));
        output.verified = false;
        assert_eq!(
            resolve_from_prior_outputs(arguments(), &[output]).unwrap_err(),
            "decision_pack_mail_prior_pack_not_verified"
        );
    }

    #[test]
    fn resolver_rejects_mismatched_output_path_set() {
        let paths = expected_paths();
        let mut mismatched = paths.clone();
        mismatched[2] = "/tmp/ship_test_02/supplier_decision.pdf".to_string();
        assert_eq!(
            resolve_from_prior_outputs(arguments(), &[pack_output(receipt(&mismatched))])
                .unwrap_err(),
            "decision_pack_mail_output_paths_mismatch"
        );
    }

    #[test]
    fn resolver_rejects_invalid_output_hash() {
        let paths = expected_paths();
        let mut invalid = receipt(&paths);
        invalid["files"][0]["sha256"] = Value::String("not-a-sha256".to_string());
        assert_eq!(
            resolve_from_prior_outputs(arguments(), &[pack_output(invalid)]).unwrap_err(),
            "decision_pack_mail_prior_pack_file_receipt_invalid"
        );
    }

    #[test]
    fn public_validator_rejects_model_supplied_body_and_bad_path_shapes() {
        let mut with_body = arguments();
        with_body["body"] = Value::String("Model supplied".to_string());
        assert!(validate_registration(with_body).is_err());

        let mut relative = arguments();
        relative["expectedOutputPaths"][0] = Value::String("supplier_decision.xlsx".to_string());
        assert!(validate_registration(relative).is_err());
    }

    #[test]
    fn resolved_preview_matcher_requires_the_complete_receipt_bound_mail() {
        let paths = expected_paths();
        let resolved = resolve_from_prior_outputs(arguments(), &[pack_output(receipt(&paths))])
            .expect("verified receipt resolves");
        let expected = paths.iter().map(String::as_str).collect::<Vec<_>>();
        assert!(resolved_preview_matches_exact_contract(
            &resolved,
            "recipient@example.com",
            "Supplier Decision Review",
            &expected,
        ));

        let mut changed_body = resolved.clone();
        changed_body["body"] = Value::String("Unbound body".to_string());
        assert!(!resolved_preview_matches_exact_contract(
            &changed_body,
            "recipient@example.com",
            "Supplier Decision Review",
            &expected,
        ));

        let mut extra_field = resolved;
        extra_field["unexpected"] = Value::Bool(true);
        assert!(!resolved_preview_matches_exact_contract(
            &extra_field,
            "recipient@example.com",
            "Supplier Decision Review",
            &expected,
        ));
    }
}
