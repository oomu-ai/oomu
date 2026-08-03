use super::{
    adapter::{self, OperationPolicy},
    auth, manifest, microsoft365, repository, ConnectorOperationRequest, ConnectorOperationResult,
    ConnectorResultSource,
};
use crate::{
    db::PersistenceEngine,
    foundation::clock::unix_time_ms_i64,
    projects::{evaluate_project_policy, ProjectTransmissionRequest},
    shield_gate::{request_user_approval, ShieldApprovalManager, ShieldApprovalRequest},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand_core::{OsRng, RngCore};
use serde_json::{json, Value};

fn random_approval_token() -> String {
    let mut bytes = [0_u8; 24];
    OsRng.fill_bytes(&mut bytes);
    format!("connector_{}", URL_SAFE_NO_PAD.encode(bytes))
}

fn operation_spec(
    manifest: &str,
    operation: &str,
) -> Result<(&'static str, bool, Vec<String>), String> {
    let spec = match (manifest, operation) {
        ("google_workspace", "gmail.search") => (
            "https://gmail.googleapis.com",
            false,
            vec!["search_query", "message_metadata"],
        ),
        ("google_workspace", "gmail.read") => (
            "https://gmail.googleapis.com",
            false,
            vec!["message_content"],
        ),
        ("google_workspace", "gmail.draft") => (
            "https://gmail.googleapis.com",
            true,
            vec!["draft_recipients", "draft_content"],
        ),
        ("google_workspace", "calendar.read") => {
            ("https://www.googleapis.com", false, vec!["calendar_events"])
        }
        ("google_workspace", "calendar.create") => {
            ("https://www.googleapis.com", true, vec!["event_details"])
        }
        ("google_workspace", "calendar.update") => {
            ("https://www.googleapis.com", true, vec!["event_details"])
        }
        ("google_workspace", "drive.search") => (
            "https://www.googleapis.com",
            false,
            vec!["search_query", "file_metadata"],
        ),
        ("google_workspace", "drive.read") => {
            ("https://www.googleapis.com", false, vec!["file_content"])
        }
        ("google_workspace", "drive.export") => (
            "https://www.googleapis.com",
            true,
            vec!["file_content", "local_export_destination"],
        ),
        ("slack", "slack.search") => (
            "https://slack.com",
            false,
            vec!["search_query", "channel_messages"],
        ),
        ("slack", "slack.thread") => ("https://slack.com", false, vec!["channel_messages"]),
        ("slack", "slack.draft") => ("local_draft", false, vec!["draft_content"]),
        ("slack", "slack.post") => (
            "https://slack.com",
            true,
            vec!["channel_destination", "message_content"],
        ),
        _ => return Err("This operation is not implemented for the connector.".to_string()),
    };
    Ok((
        spec.0,
        spec.1,
        spec.2.into_iter().map(str::to_string).collect(),
    ))
}

fn text_arg<'a>(arguments: &'a Value, key: &str, max: usize) -> Result<&'a str, String> {
    let value = arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{key} is required."))?;
    if value.len() > max {
        return Err(format!("{key} is too long."));
    }
    Ok(value)
}

fn verify_credential_binding(
    persisted: Option<&str>,
    credential: &auth::ConnectorCredential,
) -> Result<(), String> {
    match (persisted, credential.identity_binding_hash.as_deref()) {
        (Some(expected), Some(observed)) if expected == observed => Ok(()),
        (None, None) if credential.manifest_id != microsoft365::MANIFEST_ID => Ok(()),
        _ => Err("connector_identity_binding_mismatch".to_string()),
    }
}

fn require_operation_authority(
    engine: &PersistenceEngine,
    connector_id: &str,
    manifest_id: &str,
    operation: &str,
) -> Result<(), String> {
    if manifest_id == "slack" && operation == "slack.post" {
        let scopes = repository::account_granted_scopes(engine, connector_id)?;
        if !scopes.iter().any(|scope| scope == "chat:write") {
            return Err("slack_messaging_consent_required".to_string());
        }
    }
    if manifest_id == "google_workspace" {
        let granted = repository::account_granted_scopes(engine, connector_id)?;
        let required = manifest::google_required_scopes(operation)?;
        if !required.iter().all(|scope| granted.contains(scope)) {
            return Err(format!("google_incremental_consent_required:{operation}"));
        }
    }
    Ok(())
}

fn verify_adapter_postcondition(
    policy: &OperationPolicy,
    execution: &adapter::AdapterExecution,
) -> Result<(), String> {
    if policy.effectful
        && execution
            .result
            .get("mutationPostcondition")
            .and_then(Value::as_str)
            .is_none()
    {
        return Err("connector_mutation_postcondition_missing".to_string());
    }
    Ok(())
}

async fn approve_effect(
    app: &tauri::AppHandle,
    approvals: &ShieldApprovalManager,
    operation: &str,
    origin: &str,
    arguments: &Value,
    project_id: &str,
    task_run_id: Option<&str>,
) -> Result<(), String> {
    let preview = bounded_connector_approval_preview(arguments)?;
    request_user_approval(
        app,
        approvals,
        ShieldApprovalRequest {
            approval_token: random_approval_token(),
            session_id: None,
            turn_id: None,
            generation_token: None,
            action_type: "connector_write".to_string(),
            action_label: operation.to_string(),
            target_path: None,
            principal: Some(origin.to_string()),
            risk_tier: "consequential".to_string(),
            reason: "This connector operation changes data in an external account.".to_string(),
            estimated_token_costs: None,
            requested_at_ms: unix_time_ms_i64().max(0) as u64,
            preview: preview.clone(),
            semantic_summary: "Approve this connected change".to_string(),
            semantic_detail: format!(
                "Destination: {origin}. The approval is limited to the exact change shown."
            ),
            approval_tier: "effectful".to_string(),
            approval_mode: "one_time".to_string(),
            diff_preview: None,
            scope_trust_available: false,
            scope_trust_prefix: None,
            scope_trust_duration_ms: 0,
            project_id: Some(project_id.to_string()),
            task_run_id: task_run_id.map(str::to_string),
            action_class: "connector_write".to_string(),
            argument_class: crate::approval_scopes::argument_class("connector_write", &preview),
            canonical_resource: Some(origin.to_string()),
            mandatory_reconfirm: true,
            approval_scope_kinds: vec!["once".to_string()],
        },
    )
    .await
    .map_err(|error| error.message)
}

async fn approve_transmission(
    engine: &PersistenceEngine,
    app: Option<&tauri::AppHandle>,
    approvals: Option<&ShieldApprovalManager>,
    project_id: &str,
    task_id: Option<&str>,
    task_run_id: Option<&str>,
    policy: &OperationPolicy,
) -> Result<(), String> {
    let decision = |consent| ProjectTransmissionRequest {
        project_id: project_id.to_string(),
        task_id: task_id.map(str::to_string),
        destination_kind: "connector".to_string(),
        destination_origin: policy.origin.to_string(),
        data_classes: policy.data_classes.clone(),
        consent,
    };
    let preflight = evaluate_project_policy(engine, decision(false))?;
    if preflight.allowed {
        return Ok(());
    }
    if !preflight.consent_required {
        return Err("Project policy blocks this remote request.".to_string());
    }
    let app = app.ok_or_else(|| "connector_project_consent_runtime_unavailable".to_string())?;
    let approvals =
        approvals.ok_or_else(|| "connector_project_consent_runtime_unavailable".to_string())?;
    let preview=serde_json::to_string(&json!({"destination":policy.origin,"dataClasses":policy.data_classes,"policyPreview":preflight.redacted_preview})).map_err(|_|"connector_project_consent_preview_invalid".to_string())?;
    request_user_approval(
        app,
        approvals,
        ShieldApprovalRequest {
            approval_token: random_approval_token(),
            session_id: None,
            turn_id: None,
            generation_token: None,
            action_type: "connector_transmission".to_string(),
            action_label: "Use connected work data".to_string(),
            target_path: None,
            principal: Some(policy.origin.to_string()),
            risk_tier: "consequential".to_string(),
            reason: "This Project asks before connected data is sent to an external service."
                .to_string(),
            estimated_token_costs: None,
            requested_at_ms: unix_time_ms_i64().max(0) as u64,
            preview: preview.clone(),
            semantic_summary: "Allow this connected request".to_string(),
            semantic_detail: format!(
                "Send only the shown data classes to {} for this Task.",
                policy.origin
            ),
            approval_tier: "effectful".to_string(),
            approval_mode: "one_time".to_string(),
            diff_preview: None,
            scope_trust_available: false,
            scope_trust_prefix: None,
            scope_trust_duration_ms: 0,
            project_id: Some(project_id.to_string()),
            task_run_id: task_run_id.map(str::to_string),
            action_class: "connector_transmission".to_string(),
            argument_class: crate::approval_scopes::argument_class(
                "connector_transmission",
                &preview,
            ),
            canonical_resource: Some(policy.origin.to_string()),
            mandatory_reconfirm: true,
            approval_scope_kinds: vec!["once".to_string()],
        },
    )
    .await
    .map_err(|error| error.message)?;
    if evaluate_project_policy(engine, decision(true))?.allowed {
        Ok(())
    } else {
        Err("Project transmission approval was not recorded.".to_string())
    }
}

fn bounded_connector_approval_preview(arguments: &Value) -> Result<String, String> {
    const MAX_APPROVAL_PREVIEW_BYTES: usize = 256 * 1024;
    let preview = serde_json::to_string(arguments)
        .map_err(|_| "connector_approval_preview_invalid".to_string())?;
    if preview.len() > MAX_APPROVAL_PREVIEW_BYTES {
        return Err("connector_approval_preview_too_large".to_string());
    }
    Ok(preview)
}

pub(super) async fn execute(
    engine: &PersistenceEngine,
    app: Option<&tauri::AppHandle>,
    approvals: Option<&ShieldApprovalManager>,
    identity: Option<&crate::sovereign_identity::SovereignIdentity>,
    request: ConnectorOperationRequest,
) -> Result<ConnectorOperationResult, String> {
    let manifest_id =
        repository::require_project_enabled(engine, &request.connector_id, &request.project_id)?;
    let (task_id, task_run_id) = repository::validate_task_binding(
        engine,
        &request.project_id,
        request.task_id.as_deref(),
        request.task_run_id.as_deref(),
    )?;
    require_operation_authority(
        engine,
        &request.connector_id,
        &manifest_id,
        &request.operation,
    )?;
    let registered = adapter::for_manifest(&manifest_id);
    let policy = if let Some(adapter) = registered {
        adapter.operation_policy(&request.operation)?
    } else {
        let (origin, effectful, data_classes) = operation_spec(&manifest_id, &request.operation)?;
        OperationPolicy {
            origin,
            citation: origin,
            remote: origin != "local_draft",
            effectful,
            data_classes,
        }
    };
    if policy.remote {
        approve_transmission(
            engine,
            app,
            approvals,
            &request.project_id,
            task_id.as_deref(),
            task_run_id.as_deref(),
            &policy,
        )
        .await?;
    }
    if policy.effectful {
        let app = app.ok_or_else(|| "connector_approval_runtime_unavailable".to_string())?;
        let approvals =
            approvals.ok_or_else(|| "connector_approval_runtime_unavailable".to_string())?;
        let approval_arguments = if let Some(adapter) = registered {
            adapter.approval_arguments(&request.operation, &request.arguments)?
        } else {
            request.arguments.clone()
        };
        approve_effect(
            app,
            approvals,
            &request.operation,
            policy.origin,
            &approval_arguments,
            &request.project_id,
            task_run_id.as_deref(),
        )
        .await?;
    }
    let account_binding_hash = repository::identity_binding_hash(engine, &request.connector_id)?;
    let tenant_binding_hash = repository::tenant_binding_hash(engine, &request.connector_id)?;
    let (result, partial, freshness, citation) = if let Some(adapter) = registered {
        let credential = if policy.remote {
            let credential = auth::refresh_if_needed(engine, &request.connector_id, identity)?;
            verify_credential_binding(account_binding_hash.as_deref(), &credential)?;
            if microsoft365::tenant_binding_hash(credential.tenant_id.as_deref())
                != tenant_binding_hash
            {
                return Err("connector_tenant_binding_mismatch".to_string());
            }
            Some(credential)
        } else {
            None
        };
        let execution =
            adapter.execute(credential.as_ref(), &request.operation, &request.arguments)?;
        verify_adapter_postcondition(&policy, &execution)?;
        (
            execution.result,
            execution.partial,
            execution.freshness,
            execution.citation,
        )
    } else if request.operation == "slack.draft" {
        (
            json!({"channel": text_arg(&request.arguments,"channel",128)?, "text": text_arg(&request.arguments,"text",12000)?, "posted": false}),
            false,
            "local_draft",
            policy.citation.to_string(),
        )
    } else {
        let credential = auth::refresh_if_needed(engine, &request.connector_id, identity)?;
        let value = if manifest_id == "google_workspace" {
            google(
                &credential.access_token,
                &request.operation,
                &request.arguments,
            )?
        } else {
            let token = if request.operation == "slack.post" {
                credential.slack_bot_token()?
            } else {
                &credential.access_token
            };
            slack(token, &request.operation, &request.arguments)?
        };
        (value, false, "live", policy.citation.to_string())
    };
    let observed_at_ms = unix_time_ms_i64();
    Ok(ConnectorOperationResult {
        connector_id: request.connector_id,
        manifest_id,
        project_id: request.project_id,
        task_id,
        task_run_id,
        operation: request.operation,
        observed_at_ms,
        source: ConnectorResultSource {
            origin: policy.origin.to_string(),
            citation,
            freshness: freshness.to_string(),
            observed_at_ms,
        },
        account_binding_hash,
        tenant_binding_hash,
        partial,
        result,
    })
}

fn google(token: &str, operation: &str, args: &Value) -> Result<Value, String> {
    let client = reqwest::blocking::Client::new();
    let response = match operation {
        "gmail.search" => client.get("https://gmail.googleapis.com/gmail/v1/users/me/messages").bearer_auth(token).query(&[("q", text_arg(args,"query",500)?),("maxResults", "25")]).send(),
        "gmail.read" => client.get(format!("https://gmail.googleapis.com/gmail/v1/users/me/messages/{}", text_arg(args,"messageId",200)?)).bearer_auth(token).query(&[("format","full")]).send(),
        "gmail.draft" => {
            let raw = format!("To: {}\r\nSubject: {}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{}", text_arg(args,"to",1000)?, text_arg(args,"subject",500)?, text_arg(args,"body",50000)?);
            client.post("https://gmail.googleapis.com/gmail/v1/users/me/drafts").bearer_auth(token).json(&json!({"message":{"raw":URL_SAFE_NO_PAD.encode(raw.as_bytes())}})).send()
        }
        "calendar.read" => client.get("https://www.googleapis.com/calendar/v3/calendars/primary/events").bearer_auth(token).query(&[("timeMin",text_arg(args,"timeMin",64)?),("timeMax",text_arg(args,"timeMax",64)?),("maxResults","25"),("singleEvents","true")]).send(),
        "calendar.create" => client.post("https://www.googleapis.com/calendar/v3/calendars/primary/events").bearer_auth(token).json(args).send(),
        "calendar.update" => { let event_id=text_arg(args,"eventId",256)?;let payload=args.get("event").ok_or_else(||"event is required.".to_string())?;client.patch(format!("https://www.googleapis.com/calendar/v3/calendars/primary/events/{event_id}")).bearer_auth(token).json(payload).send() },
        "drive.search" => client.get("https://www.googleapis.com/drive/v3/files").bearer_auth(token).query(&[("q",text_arg(args,"query",500)?),("pageSize","25"),("fields","files(id,name,mimeType,modifiedTime,webViewLink)")]).send(),
        "drive.read" | "drive.export" => {
            let file_id = text_arg(args,"fileId",200)?;
            let downloaded = if args.get("googleDocument").and_then(Value::as_bool).unwrap_or(false) { client.get(format!("https://www.googleapis.com/drive/v3/files/{file_id}/export")).bearer_auth(token).query(&[("mimeType","text/plain")]).send() } else { client.get(format!("https://www.googleapis.com/drive/v3/files/{file_id}")).bearer_auth(token).query(&[("alt","media")]).send() };
            if operation == "drive.export" {
                let response=downloaded.map_err(|_|"connector_request_unreachable".to_string())?;if !response.status().is_success(){return Err(format!("connector_request_rejected_{}",response.status().as_u16()))}let bytes=response.bytes().map_err(|_|"connector_response_unreadable".to_string())?;if bytes.len()>10*1024*1024{return Err("connector_export_too_large".to_string())}let name=args.get("defaultFileName").and_then(Value::as_str).unwrap_or("drive-export").chars().map(|character|if character.is_ascii_alphanumeric()||matches!(character,'.'|'_'|'-'){character}else{'-'}).collect::<String>();let Some(path)=rfd::FileDialog::new().set_title("Export Google Drive File").set_file_name(&name).save_file() else{return Ok(json!({"exported":false,"cancelled":true}))};std::fs::write(&path,&bytes).map_err(|_|"connector_export_write_failed".to_string())?;return Ok(json!({"exported":true,"fileName":path.file_name().and_then(|value|value.to_str()).unwrap_or("export")}));
            }
            downloaded
        }
        _ => return Err("Unsupported Google operation.".to_string()),
    }.map_err(|_| "connector_request_unreachable".to_string())?;
    let status = response.status();
    let bytes = response
        .bytes()
        .map_err(|_| "connector_response_unreadable".to_string())?;
    if !status.is_success() {
        return Err(format!("connector_request_rejected_{}", status.as_u16()));
    }
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("connector_response_too_large".to_string());
    }
    if let Ok(value) = serde_json::from_slice(&bytes) {
        return Ok(value);
    }
    String::from_utf8(bytes.to_vec())
        .map(|text| json!({"text":text}))
        .map_err(|_| "connector_response_invalid".to_string())
}

fn slack(token: &str, operation: &str, args: &Value) -> Result<Value, String> {
    let client = reqwest::blocking::Client::new();
    let response = match operation {
        "slack.search" => client.get("https://slack.com/api/search.messages").bearer_auth(token).query(&[("query",text_arg(args,"query",500)?),("count","25")]).send(),
        "slack.thread" => client.get("https://slack.com/api/conversations.replies").bearer_auth(token).query(&[("channel",text_arg(args,"channel",128)?),("ts",text_arg(args,"threadTs",64)?),("limit","50")]).send(),
        "slack.post" => client.post("https://slack.com/api/chat.postMessage").bearer_auth(token).json(&json!({"channel":text_arg(args,"channel",128)?,"text":text_arg(args,"text",12000)?,"thread_ts":args.get("threadTs")})).send(),
        _ => return Err("Unsupported Slack operation.".to_string()),
    }.map_err(|_| "connector_request_unreachable".to_string())?;
    let status = response.status();
    let body: Value = response
        .json()
        .map_err(|_| "connector_response_invalid".to_string())?;
    if !status.is_success() || body.get("ok").is_some_and(|value| value == false) {
        return Err(body
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("connector_request_rejected")
            .to_string());
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn effectful_operations_are_explicit() {
        assert!(operation_spec("google_workspace", "gmail.draft").unwrap().1);
        assert!(operation_spec("slack", "slack.post").unwrap().1);
        assert!(!operation_spec("slack", "slack.search").unwrap().1);
        let microsoft = adapter::for_manifest(microsoft365::MANIFEST_ID).unwrap();
        assert!(
            microsoft
                .operation_policy(microsoft365::OUTLOOK_MAIL_DRAFT)
                .unwrap()
                .effectful
        );
        assert!(
            !microsoft
                .operation_policy(microsoft365::TEAMS_DRAFT)
                .unwrap()
                .effectful
        );
    }

    #[test]
    fn read_only_slack_cannot_reach_the_send_approval_boundary() {
        let root = std::env::temp_dir().join(format!(
            "oomu-slack-read-only-{}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let connector_id = repository::create_account(&engine, "slack", 1).unwrap();
        engine.open_connection().unwrap().execute(
            "UPDATE connector_accounts SET connection_state='authorized',granted_scopes_json='[\"channels:read\"]' WHERE connector_id=?1",
            rusqlite::params![connector_id],
        ).unwrap();

        assert_eq!(
            require_operation_authority(&engine, &connector_id, "slack", "slack.post").unwrap_err(),
            "slack_messaging_consent_required"
        );
        assert!(
            require_operation_authority(&engine, &connector_id, "slack", "slack.search").is_ok()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn google_operation_requires_only_its_staged_scope() {
        let root = std::env::temp_dir().join(format!(
            "oomu-google-operation-scope-{}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let connector_id = repository::create_account(
            &engine,
            "google_workspace",
            manifest::manifest("google_workspace").unwrap().version,
        )
        .unwrap();
        engine.open_connection().unwrap().execute(
            "UPDATE connector_accounts
             SET connection_state='authorized',
                 granted_scopes_json='[\"openid\",\"email\",\"profile\",\"https://www.googleapis.com/auth/calendar.readonly\"]'
             WHERE connector_id=?1",
            rusqlite::params![connector_id],
        ).unwrap();

        assert!(require_operation_authority(
            &engine,
            &connector_id,
            "google_workspace",
            "calendar.read",
        )
        .is_ok());
        assert_eq!(
            require_operation_authority(&engine, &connector_id, "google_workspace", "gmail.read",)
                .unwrap_err(),
            "google_incremental_consent_required:gmail.read",
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn generic_adapter_boundary_enforces_identity_binding_and_local_non_delivery() {
        let credential = auth::ConnectorCredential {
            manifest_id: microsoft365::MANIFEST_ID.to_string(),
            access_token: "token".to_string(),
            bot_access_token: None,
            refresh_token: Some("refresh".to_string()),
            token_type: "Bearer".to_string(),
            scopes: vec!["User.Read".to_string()],
            expires_at_ms: None,
            refresh_expires_at_ms: None,
            tenant_id: Some("tenant".to_string()),
            tenant_label: None,
            account_id: Some("account".to_string()),
            account_principal: Some("person@example.com".to_string()),
            identity_binding_hash: Some("binding-a".to_string()),
        };
        verify_credential_binding(Some("binding-a"), &credential).unwrap();
        assert_eq!(
            verify_credential_binding(Some("binding-b"), &credential).unwrap_err(),
            "connector_identity_binding_mismatch"
        );
        let adapter = adapter::for_manifest(microsoft365::MANIFEST_ID).unwrap();
        let execution = adapter
            .execute(
                None,
                microsoft365::TEAMS_DRAFT,
                &json!({"chatId":"chat-1","text":"Draft only"}),
            )
            .unwrap();
        assert_eq!(execution.result["posted"], false);
        assert_eq!(execution.freshness, "local_draft");
        assert!(execution.citation.starts_with("local://teams/chat/"));
        assert!(execution.citation.ends_with("/draft"));
        assert!(!execution.citation.contains("chat-1"));

        let effect_policy = OperationPolicy {
            origin: "https://example.invalid",
            citation: "connector://mutation",
            remote: true,
            effectful: true,
            data_classes: vec![],
        };
        let missing = adapter::AdapterExecution {
            result: json!({"id":"one"}),
            partial: false,
            freshness: "live",
            citation: "connector://mutation/one".to_string(),
        };
        assert_eq!(
            verify_adapter_postcondition(&effect_policy, &missing).unwrap_err(),
            "connector_mutation_postcondition_missing"
        );
        let observed = adapter::AdapterExecution {
            result: json!({"id":"one","mutationPostcondition":"observed"}),
            ..missing
        };
        verify_adapter_postcondition(&effect_policy, &observed).unwrap();
    }

    #[test]
    fn connector_approval_preview_is_exact_json_secret_free_and_bounded() {
        let microsoft = adapter::for_manifest(microsoft365::MANIFEST_ID).unwrap();
        let normalized = microsoft
            .approval_arguments(
                microsoft365::OUTLOOK_MAIL_DRAFT,
                &json!({
                    "to":["person@example.com"],
                    "cc":["reviewer@example.com"],
                    "subject":"Quarterly review",
                    "body":"Please review the attached summary.",
                    "accessToken":"credential-canary",
                    "refreshToken":"refresh-canary"
                }),
            )
            .unwrap();
        let preview = bounded_connector_approval_preview(&normalized).unwrap();
        let displayed: Value = serde_json::from_str(&preview).unwrap();
        assert_eq!(displayed["to"], json!(["person@example.com"]));
        assert_eq!(displayed["cc"], json!(["reviewer@example.com"]));
        assert_eq!(displayed["subject"], "Quarterly review");
        assert_eq!(displayed["body"], "Please review the attached summary.");
        assert!(!preview.contains("credential-canary"));
        assert!(!preview.contains("refresh-canary"));

        let too_large = json!({"body":"x".repeat(256 * 1024)});
        assert_eq!(
            bounded_connector_approval_preview(&too_large).unwrap_err(),
            "connector_approval_preview_too_large"
        );
    }

    #[test]
    fn renderer_cannot_self_assert_project_transmission_consent() {
        let attempted = serde_json::from_value::<ConnectorOperationRequest>(json!({
            "connectorId":"connector_11111111-1111-4111-8111-111111111111",
            "projectId":"project_22222222-2222-4222-8222-222222222222",
            "operation":"outlook.mail.search",
            "arguments":{"query":"quarterly"},
            "projectConsent":true
        }));
        assert!(attempted.is_err());
    }
}
