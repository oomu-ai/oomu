use super::*;
use crate::{
    db::PersistenceEngine,
    foundation::clock::unix_time_ms_i64,
    gemma::{GemmaService, GemmaStatus},
    shield_gate::ShieldApprovalManager,
};
use tauri_plugin_notification::{NotificationExt, PermissionState};
use tauri_plugin_opener::OpenerExt;

async fn blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn list_connector_manifests(
    mcp: tauri::State<'_, crate::mcp::client::McpClientRegistry>,
) -> Result<Vec<ConnectorManifest>, String> {
    let mut manifests = manifest::manifests();
    if let Some(runtime) = manifests
        .iter_mut()
        .find(|item| item.manifest_id == "mcp_runtime")
    {
        if let Ok(configs) = crate::mcp::bootstrap::mcp_builtin_server_configs_headless() {
            for config in configs {
                if let Ok(tools) = mcp.list_tools(&config.name).await {
                    runtime.tools.extend(tools.into_iter().map(|tool| {
                        ConnectorTool {
                            name: format!("{} / {}", config.name, tool.name),
                            risk: tool
                                .annotations
                                .as_ref()
                                .and_then(|value| value.get("readOnlyHint"))
                                .and_then(serde_json::Value::as_bool)
                                .filter(|value| *value)
                                .map(|_| "read")
                                .unwrap_or("review")
                                .to_string(),
                            description: tool.description,
                            input_schema: tool.input_schema,
                            output_schema: tool.output_schema,
                        }
                    }));
                }
            }
        }
    }
    Ok(manifests)
}

#[tauri::command]
pub async fn list_connector_accounts(
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Vec<ConnectorAccount>, String> {
    let engine = persistence.inner().clone();
    blocking(move || repository::list_accounts(&engine)).await
}

#[tauri::command]
pub async fn get_connector_connection_status(
    request: ConnectorIdRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<ConnectorConnectionStatus, String> {
    let engine = persistence.inner().clone();
    blocking(move || repository::connection_status(&engine, &request.connector_id)).await
}

#[tauri::command]
pub async fn list_slack_conversations(
    request: ConnectorIdRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    identity: tauri::State<'_, crate::sovereign_identity::SovereignIdentity>,
) -> Result<Vec<SlackConversation>, String> {
    let engine = persistence.inner().clone();
    let identity = identity.inner().clone();
    blocking(move || {
        let access = super::slack_gateway_credential(&engine, &request.connector_id, &identity)?;
        fetch_slack_conversations(&access.bot_access_token)
    })
    .await
}

fn fetch_slack_conversations(bot_token: &str) -> Result<Vec<SlackConversation>, String> {
    let client = reqwest::blocking::Client::new();
    let mut cursor = String::new();
    let mut conversations = Vec::new();
    for _ in 0..5 {
        let mut request = client
            .get("https://slack.com/api/conversations.list")
            .bearer_auth(bot_token)
            .query(&[
                ("types", "public_channel,private_channel,mpim,im"),
                ("exclude_archived", "true"),
                ("limit", "200"),
            ]);
        if !cursor.is_empty() {
            request = request.query(&[("cursor", cursor.as_str())]);
        }
        let response = request
            .send()
            .map_err(|_| "slack_conversations_unreachable".to_string())?;
        let status = response.status();
        let bytes = response
            .bytes()
            .map_err(|_| "slack_conversations_invalid".to_string())?;
        if bytes.len() > 2 * 1024 * 1024 {
            return Err("slack_conversations_invalid".to_string());
        }
        let payload: Value = serde_json::from_slice(&bytes)
            .map_err(|_| "slack_conversations_invalid".to_string())?;
        if !status.is_success() || payload.get("ok").and_then(Value::as_bool) != Some(true) {
            return Err(payload
                .get("error")
                .and_then(Value::as_str)
                .filter(|code| {
                    matches!(
                        *code,
                        "missing_scope" | "not_authed" | "invalid_auth" | "token_revoked"
                    )
                })
                .unwrap_or("slack_conversations_rejected")
                .to_string());
        }
        for item in payload
            .get("channels")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(id) = item
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| {
                    !id.is_empty()
                        && id.len() <= 128
                        && id.bytes().all(|byte| byte.is_ascii_alphanumeric())
                })
            else {
                continue;
            };
            let is_im = item.get("is_im").and_then(Value::as_bool) == Some(true);
            let is_mpim = item.get("is_mpim").and_then(Value::as_bool) == Some(true);
            let is_private = item.get("is_private").and_then(Value::as_bool) == Some(true);
            let joined = item.get("is_member").and_then(Value::as_bool) == Some(true);
            if !joined && !is_im && !is_mpim {
                continue;
            }
            let name = item
                .get("name_normalized")
                .or_else(|| item.get("name"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(|name| name.chars().take(100).collect::<String>());
            conversations.push(SlackConversation {
                id: id.to_string(),
                name,
                kind: if is_im {
                    "direct_message"
                } else if is_mpim {
                    "group_message"
                } else if is_private {
                    "private_channel"
                } else {
                    "channel"
                }
                .to_string(),
            });
        }
        cursor = payload
            .pointer("/response_metadata/next_cursor")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .to_string();
        if cursor.is_empty() {
            break;
        }
    }
    conversations.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.id.cmp(&right.id))
    });
    conversations.dedup_by(|left, right| left.id == right.id);
    Ok(conversations)
}

#[tauri::command]
pub async fn begin_connector_oauth(
    request: BeginOAuthRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    identity: tauri::State<'_, crate::sovereign_identity::SovereignIdentity>,
    app: tauri::AppHandle,
) -> Result<BeginOAuthResponse, String> {
    let engine = persistence.inner().clone();
    let identity = identity.inner().clone();
    let response = blocking(move || {
        auth::begin(
            &engine,
            &identity,
            request.manifest_id.trim(),
            request.connector_id.as_deref(),
            &request.requested_operations,
        )
    })
    .await?;
    app.opener()
        .open_url(&response.authorization_url, None::<&str>)
        .map_err(|error| format!("Unable to open the authorization page: {error}"))?;
    Ok(response)
}

#[tauri::command]
pub async fn set_connector_project_scope(
    request: SetConnectorProjectScopeRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<ConnectorProjectScope, String> {
    let engine = persistence.inner().clone();
    blocking(move || repository::set_project_scope(&engine, request)).await
}

#[tauri::command]
pub async fn test_connector(
    request: ConnectorIdRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    mcp: tauri::State<'_, crate::mcp::client::McpClientRegistry>,
    identity: tauri::State<'_, crate::sovereign_identity::SovereignIdentity>,
) -> Result<CapabilityHealth, String> {
    let engine = persistence.inner().clone();
    let identity = identity.inner().clone();
    let manifest_id = repository::account_manifest(&engine, &request.connector_id)?;
    if manifest_id == "mcp_runtime" {
        let configs = crate::mcp::bootstrap::mcp_builtin_server_configs_headless()?;
        let mut tool_count = 0usize;
        for config in configs {
            tool_count += mcp
                .list_tools(&config.name)
                .await
                .map_err(|error| error.to_string())?
                .len();
        }
        repository::record_probe(
            &engine,
            &request.connector_id,
            "reachable",
            "tool_schema_probe_ok",
            None,
        )?;
        return Ok(CapabilityHealth {
            capability_id: request.connector_id,
            state: "reachable".to_string(),
            detail: format!("{tool_count} MCP tool schema(s) passed a live registry probe."),
            detail_code: Some("tool_schema_probe_ok".to_string()),
            repair_action: None,
            repair_action_code: None,
            checked_at_ms: unix_time_ms_i64(),
        });
    }
    if manifest_id == "apple_apps" {
        #[cfg(target_os = "macos")]
        let probe = std::process::Command::new("/usr/bin/osascript")
            .args(["-e", "tell application \"Calendar\" to get name"])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false);
        #[cfg(not(target_os = "macos"))]
        let probe = false;
        let state = if probe { "reachable" } else { "blocked" };
        let code = if probe {
            "automation_probe_ok"
        } else {
            "automation_permission_blocked"
        };
        repository::record_probe(&engine, &request.connector_id, state, code, None)?;
        return Ok(CapabilityHealth {
            capability_id: request.connector_id,
            state: state.to_string(),
            detail: if probe {
                "Apple Calendar responded to a live Automation probe.".to_string()
            } else {
                "macOS did not allow the selected Apple app automation probe.".to_string()
            },
            detail_code: Some(code.to_string()),
            repair_action: (!probe).then(|| {
                "Allow OOMU to control the selected Apple app in System Settings.".to_string()
            }),
            repair_action_code: (!probe).then(|| "open_macos_automation_settings".to_string()),
            checked_at_ms: unix_time_ms_i64(),
        });
    }
    blocking(move || {
        let credential =
            match auth::refresh_if_needed(&engine, &request.connector_id, Some(&identity)) {
                Ok(value) => value,
                Err(error) => {
                    let (state, repair_code) = connector_diagnostic(&error);
                    let _ = repository::record_probe(
                        &engine,
                        &request.connector_id,
                        state,
                        &error,
                        None,
                    );
                    return Ok(CapabilityHealth {
                        capability_id: request.connector_id,
                        state: state.to_string(),
                        detail: "The account authorization needs attention.".to_string(),
                        detail_code: Some(error),
                        repair_action: Some("Reconnect the account.".to_string()),
                        repair_action_code: Some(repair_code.to_string()),
                        checked_at_ms: unix_time_ms_i64(),
                    });
                }
            };
        match auth::probe_identity(&credential) {
            Ok(_) => {
                repository::record_probe(
                    &engine,
                    &request.connector_id,
                    "reachable",
                    "identity_probe_ok",
                    credential.expires_at_ms,
                )?;
                Ok(CapabilityHealth {
                    capability_id: request.connector_id,
                    state: "reachable".to_string(),
                    detail: "The account responded to a live identity probe.".to_string(),
                    detail_code: Some("connector_identity_probe_ok".to_string()),
                    repair_action: None,
                    repair_action_code: None,
                    checked_at_ms: unix_time_ms_i64(),
                })
            }
            Err(code) => {
                let (state, repair_code) = connector_diagnostic(&code);
                repository::record_probe(
                    &engine,
                    &request.connector_id,
                    state,
                    &code,
                    credential.expires_at_ms,
                )?;
                Ok(CapabilityHealth {
                    capability_id: request.connector_id,
                    state: state.to_string(),
                    detail: "The account did not pass its live probe.".to_string(),
                    detail_code: Some(code),
                    repair_action: Some(
                        "Test the network, then reconnect if the problem continues.".to_string(),
                    ),
                    repair_action_code: Some(repair_code.to_string()),
                    checked_at_ms: unix_time_ms_i64(),
                })
            }
        }
    })
    .await
}

fn connector_diagnostic(code: &str) -> (&'static str, &'static str) {
    if code.contains("revoked") || code.contains("refresh_token_missing") {
        ("expired", "connector_reconnect")
    } else if code.contains("tenant_policy") {
        ("blocked", "connector_tenant_admin_review")
    } else if code.contains("rate_limited") {
        ("degraded", "connector_retry_later")
    } else if code.contains("offline") || code.contains("unavailable") {
        ("degraded", "connector_check_network")
    } else {
        ("degraded", "connector_reconnect")
    }
}

#[tauri::command]
pub async fn disconnect_connector(
    request: ConnectorIdRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<(), String> {
    let engine = persistence.inner().clone();
    blocking(move || {
        auth::revoke(&request.connector_id)?;
        repository::disconnect(&engine, &request.connector_id)
    })
    .await
}

#[tauri::command]
pub async fn execute_connector_operation(
    request: ConnectorOperationRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    approvals: tauri::State<'_, ShieldApprovalManager>,
    identity: tauri::State<'_, crate::sovereign_identity::SovereignIdentity>,
    app: tauri::AppHandle,
) -> Result<ConnectorOperationResult, String> {
    api::execute(
        persistence.inner(),
        Some(&app),
        Some(approvals.inner()),
        Some(identity.inner()),
        request,
    )
    .await
}

#[tauri::command]
pub async fn get_capability_health(
    persistence: tauri::State<'_, PersistenceEngine>,
    gemma: tauri::State<'_, GemmaService>,
    app: tauri::AppHandle,
) -> Result<Vec<CapabilityHealth>, String> {
    let now = unix_time_ms_i64();
    let model_state = match gemma.get_status() {
        GemmaStatus::Ready => ("reachable", "The local model is ready.", None),
        GemmaStatus::Loading => (
            "configured",
            "The local model is still preparing.",
            Some("Wait for model preparation to finish."),
        ),
        GemmaStatus::Degraded => (
            "degraded",
            "The local model did not pass its runtime probe.",
            Some("Repair or select the local model files."),
        ),
        GemmaStatus::Shutdown => (
            "blocked",
            "The local model runtime is stopped.",
            Some("Restart OOMU."),
        ),
    };
    let notification = match app.notification().permission_state() {
        Ok(PermissionState::Granted) => ("reachable", "Notifications are available.", None),
        Ok(PermissionState::Denied) => (
            "blocked",
            "Notifications are disabled in macOS.",
            Some("Allow notifications in System Settings."),
        ),
        Ok(_) => (
            "configured",
            "Notifications have not been requested.",
            Some("Enable notifications when you choose a completion channel."),
        ),
        Err(_) => (
            "degraded",
            "Notification status could not be checked.",
            Some("Open System Settings and verify notifications."),
        ),
    };
    let keychain = match crate::keychain_session::status() {
        crate::keychain_session::SessionStatus::Available => (
            "reachable",
            "Keychain credential access is available for this session.",
            None,
        ),
        crate::keychain_session::SessionStatus::Unverified => (
            "configured",
            "Keychain access has not been needed in this session.",
            None,
        ),
        crate::keychain_session::SessionStatus::Unavailable => (
            "blocked",
            "Keychain credential access is unavailable.",
            Some("Unlock Keychain and allow OOMU access."),
        ),
    };
    let mut health = vec![
        health("local_model", model_state, now),
        health("keychain", keychain, now),
        health("notifications", notification, now),
    ];
    health.extend(macos_permission_health(now));
    if let Ok(connection) = persistence.open_connection() {
        let (sources,revoked):(i64,i64)=connection.query_row("SELECT COUNT(*),COALESCE(SUM(CASE WHEN length(trim(grant_reference))=0 OR grant_state!='active' THEN 1 ELSE 0 END),0) FROM project_sources",[],|row|Ok((row.get(0)?,row.get(1)?))).unwrap_or((0,0));
        health.push(CapabilityHealth {
            capability_id: "project_files".to_string(),
            state: if revoked > 0 {
                "blocked"
            } else if sources > 0 {
                "reachable"
            } else {
                "configured"
            }
            .to_string(),
            detail: if revoked > 0 {
                format!("{revoked} Project folder grant(s) need repair.")
            } else if sources > 0 {
                format!("{sources} Project folder grant(s) are reachable.")
            } else {
                "No Project folders need access yet.".to_string()
            },
            detail_code: Some(
                if revoked > 0 {
                    "project_folder_grant_repair_required"
                } else if sources > 0 {
                    "project_folder_grants_reachable"
                } else {
                    "project_folder_grants_unused"
                }
                .to_string(),
            ),
            repair_action: (revoked > 0)
                .then(|| "Re-select the affected Project folder.".to_string()),
            repair_action_code: (revoked > 0).then(|| "project_folder_reselect".to_string()),
            checked_at_ms: now,
        });
    }
    if let Ok(status) = crate::routines::background::status(persistence.inner()) {
        health.push(CapabilityHealth {
            capability_id: "background_service".to_string(),
            state: status.state,
            detail: status.detail,
            detail_code: Some("background_service_status".to_string()),
            repair_action: None,
            repair_action_code: None,
            checked_at_ms: status.checked_at_ms,
        });
    }
    for account in repository::list_accounts(persistence.inner())? {
        let detail_code = account.last_probe_code.clone();
        health.push(CapabilityHealth {
            capability_id: account.connector_id,
            state: account.connection_state,
            detail: account
                .last_probe_code
                .unwrap_or_else(|| "Run a live connection test.".to_string()),
            detail_code,
            repair_action: Some("Test or reconnect this account.".to_string()),
            repair_action_code: Some("connector_test_or_reconnect".to_string()),
            checked_at_ms: account.last_probe_at_ms.unwrap_or(now),
        });
    }
    Ok(health)
}

fn health(id: &str, value: (&str, &str, Option<&str>), checked: i64) -> CapabilityHealth {
    CapabilityHealth {
        capability_id: id.to_string(),
        state: value.0.to_string(),
        detail: value.1.to_string(),
        detail_code: None,
        repair_action: value.2.map(str::to_string),
        repair_action_code: None,
        checked_at_ms: checked,
    }
}

#[cfg(target_os = "macos")]
fn macos_permission_health(now: i64) -> Vec<CapabilityHealth> {
    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> bool;
        fn CGPreflightScreenCaptureAccess() -> bool;
    }
    let accessibility = unsafe { AXIsProcessTrusted() };
    let screen = unsafe { CGPreflightScreenCaptureAccess() };
    vec![
        CapabilityHealth {
            capability_id: "macos_accessibility".to_string(),
            state: if accessibility {
                "reachable"
            } else {
                "blocked"
            }
            .to_string(),
            detail: if accessibility {
                "Accessibility access is available."
            } else {
                "Accessibility access is not granted."
            }
            .to_string(),
            detail_code: Some(
                if accessibility {
                    "macos_accessibility_reachable"
                } else {
                    "macos_accessibility_blocked"
                }
                .to_string(),
            ),
            repair_action: (!accessibility)
                .then(|| "Allow OOMU in Privacy & Security, Accessibility.".to_string()),
            repair_action_code: (!accessibility)
                .then(|| "open_macos_accessibility_settings".to_string()),
            checked_at_ms: now,
        },
        CapabilityHealth {
            capability_id: "macos_screen_recording".to_string(),
            state: if screen { "reachable" } else { "blocked" }.to_string(),
            detail: if screen {
                "Screen Recording access is available."
            } else {
                "Screen Recording access is not granted."
            }
            .to_string(),
            detail_code: Some(
                if screen {
                    "macos_screen_recording_reachable"
                } else {
                    "macos_screen_recording_blocked"
                }
                .to_string(),
            ),
            repair_action: (!screen)
                .then(|| "Allow OOMU in Privacy & Security, Screen Recording.".to_string()),
            repair_action_code: (!screen)
                .then(|| "open_macos_screen_recording_settings".to_string()),
            checked_at_ms: now,
        },
    ]
}
#[cfg(not(target_os = "macos"))]
fn macos_permission_health(now: i64) -> Vec<CapabilityHealth> {
    vec![CapabilityHealth {
        capability_id: "macos_permissions".to_string(),
        state: "unsupported".to_string(),
        detail: "macOS permissions are unavailable on this system.".to_string(),
        detail_code: Some("macos_permissions_unsupported".to_string()),
        repair_action: None,
        repair_action_code: None,
        checked_at_ms: now,
    }]
}
