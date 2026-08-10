use super::*;

pub async fn mcp_execute_tool(
    server_name: String,
    tool_name: String,
    arguments: Value,
    approval: Option<McpToolApproval>,
    approval_scope_kind: Option<String>,
    turn_context: Option<McpChatTurnContext>,
    registry: tauri::State<'_, McpClientRegistry>,
    persistence: tauri::State<'_, PersistenceEngine>,
    app: tauri::AppHandle,
) -> Result<McpToolCallResult, String> {
    validate_reusable_approval_scope(&server_name, &tool_name, approval_scope_kind.as_deref())?;
    let turn_context = turn_context.map(ChatTurnPersistenceContext::from);
    let persistence = persistence.inner().clone();
    let guard = || validate_mcp_chat_turn(&persistence, turn_context.as_ref());
    if let Some(result) = execute_project_file_read_if_supported(
        &persistence,
        turn_context.as_ref(),
        &server_name,
        &tool_name,
        &arguments,
    )
    .await
    {
        return result;
    }
    if let Some(result) = native_public_search_execution::execute_if_supported(
        &server_name,
        &tool_name,
        &arguments,
        approval.clone(),
        approval_scope_kind.as_deref(),
        turn_context.as_ref(),
        &persistence,
        registry.inner(),
        &app,
        &guard,
    )
    .await
    {
        return result;
    }
    if let Some(result) = native_capability_execution::execute_if_supported(
        &server_name,
        &tool_name,
        &arguments,
        approval.clone(),
        turn_context.as_ref(),
        &persistence,
        registry.inner(),
        &app,
        &guard,
    )
    .await
    {
        return result;
    }
    if let Some(result) = execute_direct_apple_read_if_supported(
        &server_name,
        &tool_name,
        &arguments,
        turn_context.as_ref(),
        &persistence,
        registry.inner(),
        &app,
    )
    .await
    {
        return result;
    }
    let receipt = native_apple_receipts::spec_for(&server_name, &tool_name, &arguments);
    let action_approved = approval.is_some();
    native_apple_receipts::execute(
        receipt,
        turn_context.as_ref(),
        &persistence,
        action_approved,
        async {
            registry
                .execute_tool_with_approval_guarded(
                    &server_name,
                    &tool_name,
                    arguments,
                    approval,
                    &guard,
                )
                .await
                .map_err(|error| error.message)
        },
    )
    .await
}

async fn execute_project_file_read_if_supported(
    persistence: &PersistenceEngine,
    turn_context: Option<&ChatTurnPersistenceContext>,
    server_name: &str,
    tool_name: &str,
    arguments: &Value,
) -> Option<Result<McpToolCallResult, String>> {
    let project_id = match conversational_project_file_read_project_id(
        persistence,
        turn_context,
        server_name,
        tool_name,
    ) {
        Ok(Some(project_id)) => project_id,
        Ok(None) => return None,
        Err(error) => return Some(Err(error)),
    };
    let result = async {
        validate_tool_arguments(arguments).map_err(|error| error.message)?;
        let path = conversational_project_file_read_path(arguments)?;
        validate_mcp_chat_turn(persistence, turn_context).map_err(|error| error.message)?;
        let receipt = native_apple_receipts::spec_for(server_name, tool_name, arguments);
        let execution_persistence = persistence.clone();
        native_apple_receipts::execute(receipt, turn_context, persistence, false, async move {
            conversational_project_file_read(&execution_persistence, &project_id, &path)
        })
        .await
    }
    .await;
    Some(result)
}

async fn execute_direct_apple_read_if_supported(
    server_name: &str,
    tool_name: &str,
    arguments: &Value,
    turn_context: Option<&ChatTurnPersistenceContext>,
    persistence: &PersistenceEngine,
    registry: &McpClientRegistry,
    app: &tauri::AppHandle,
) -> Option<Result<McpToolCallResult, String>> {
    if is_system_calendar_tool(server_name, tool_name) {
        let result = async {
            validate_guarded_tool_arguments(arguments, persistence, turn_context)?;
            let (calendar_name, hours_ahead, start_date, end_date) =
                bounded_system_calendar_arguments(arguments)?;
            native_apple_receipts::execute(
                native_apple_receipts::spec_for(server_name, tool_name, arguments),
                turn_context,
                persistence,
                true,
                read_system_calendar_with_deadline(
                    calendar_name,
                    hours_ahead,
                    start_date,
                    end_date,
                    registry,
                    app,
                ),
            )
            .await
        }
        .await;
        return Some(result);
    }
    if is_system_contacts_tool(server_name, tool_name) {
        let result = async {
            validate_guarded_tool_arguments(arguments, persistence, turn_context)?;
            let request = crate::tools::system_contacts::contact_request_from_arguments(arguments)?;
            native_apple_receipts::execute(
                native_apple_receipts::spec_for(server_name, tool_name, arguments),
                turn_context,
                persistence,
                true,
                read_system_contacts_with_fallback(request, registry, app),
            )
            .await
        }
        .await;
        return Some(result);
    }
    if is_system_photos_tool(server_name, tool_name) {
        let result = async {
            validate_guarded_tool_arguments(arguments, persistence, turn_context)?;
            let max_photos = crate::system_photos::photo_limit_from_arguments(arguments)?;
            native_apple_receipts::execute(
                native_apple_receipts::spec_for(server_name, tool_name, arguments),
                turn_context,
                persistence,
                true,
                async move {
                    Ok(crate::system_photos::read_system_photos_bounded(max_photos).await)
                },
            )
            .await
        }
        .await;
        return Some(result);
    }
    if is_system_music_tool(server_name, tool_name) {
        let result = async {
            validate_guarded_tool_arguments(arguments, persistence, turn_context)?;
            let max_songs = crate::system_music::song_limit_from_arguments(arguments)?;
            native_apple_receipts::execute(
                native_apple_receipts::spec_for(server_name, tool_name, arguments),
                turn_context,
                persistence,
                true,
                async move { Ok(crate::system_music::read_system_music_bounded(max_songs).await) },
            )
            .await
        }
        .await;
        return Some(result);
    }
    if system_mail::is_system_mail_read_tool(server_name, tool_name) {
        return Some(
            native_apple_receipts::execute(
                native_apple_receipts::spec_for(server_name, tool_name, arguments),
                turn_context,
                persistence,
                true,
                system_mail::execute_turn_bound_mail_read(
                    arguments.clone(),
                    turn_context,
                    registry,
                    persistence,
                    app,
                ),
            )
            .await,
        );
    }
    None
}

fn validate_guarded_tool_arguments(
    arguments: &Value,
    persistence: &PersistenceEngine,
    turn_context: Option<&ChatTurnPersistenceContext>,
) -> Result<(), String> {
    validate_tool_arguments(arguments).map_err(|error| error.message)?;
    validate_mcp_chat_turn(persistence, turn_context).map_err(|error| error.message)
}

fn is_project_file_read_tool(server_name: &str, tool_name: &str) -> bool {
    server_name.trim().eq_ignore_ascii_case("local_filesystem")
        && tool_name.trim().eq_ignore_ascii_case("read_file")
}

pub(super) fn conversational_project_file_read_project_id(
    persistence: &PersistenceEngine,
    turn_context: Option<&ChatTurnPersistenceContext>,
    server_name: &str,
    tool_name: &str,
) -> Result<Option<String>, String> {
    if !is_project_file_read_tool(server_name, tool_name) {
        return Ok(None);
    }
    let Some(turn) = turn_context else {
        return Ok(None);
    };
    Ok(persistence
        .project_inference_context_for_session(&turn.session_id)?
        .map(|project| project.project_id))
}

pub(super) fn conversational_project_file_read_path(arguments: &Value) -> Result<String, String> {
    let object = arguments
        .as_object()
        .filter(|object| object.len() == 1 && object.contains_key("path"))
        .ok_or_else(|| "read_file accepts only one path.".to_string())?;
    object
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "read_file requires a path.".to_string())
}

pub(super) fn conversational_project_file_read(
    persistence: &PersistenceEngine,
    project_id: &str,
    path: &str,
) -> Result<McpToolCallResult, String> {
    let receipt = crate::tools::project_file::read_project_file(
        persistence,
        project_id,
        path,
        8 * 1024 * 1024,
    )?;
    let canonical_path = receipt.canonical_path;
    let relative_path = crate::tools::project_file::relative_path_in_active_project(
        persistence,
        project_id,
        &canonical_path,
    )?;
    let content = receipt.content;
    Ok(McpToolCallResult {
        content: vec![serde_json::json!({
            "type": "text",
            "text": content,
        })],
        structured_content: Some(serde_json::json!({
            "code": "project_file_read_ok",
            "path": relative_path,
            "relativePath": relative_path,
            "content": content,
            "byteCount": receipt.byte_count,
            "contentSha256": receipt.content_sha256,
            "verified": receipt.verified,
        })),
        is_error: false,
        meta: None,
        raw: None,
    })
}

fn validate_reusable_approval_scope(
    server_name: &str,
    tool_name: &str,
    approval_scope_kind: Option<&str>,
) -> Result<(), String> {
    let requests_reusable_scope = approval_scope_kind
        .map(str::trim)
        .filter(|scope| !scope.is_empty())
        .is_some_and(|scope| scope != "once");
    if requests_reusable_scope
        && !native_public_search_execution::is_supported_tool(server_name, tool_name)
    {
        return Err("Reusable approval is not available for this MCP tool.".to_string());
    }
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn mcp_prepare_tool_approval(
    server_name: String,
    tool_name: String,
    arguments: serde_json::Value,
    turn_context: Option<McpChatTurnContext>,
    registry: tauri::State<'_, McpClientRegistry>,
    persistence: tauri::State<'_, PersistenceEngine>,
    approvals: tauri::State<'_, ShieldApprovalManager>,
    app: tauri::AppHandle,
) -> Result<Option<McpToolApprovalRequest>, String> {
    public_search_session_approval::prepare_tool_approval(
        server_name,
        tool_name,
        arguments,
        turn_context,
        registry,
        persistence,
        approvals,
        app,
    )
    .await
}

pub async fn prepare_system_apple_app_tool_approval(
    tool_name: String,
    arguments: Value,
    registry: tauri::State<'_, McpClientRegistry>,
    app: tauri::AppHandle,
) -> Result<Option<McpToolApprovalRequest>, String> {
    let tool_name = normalize_system_apple_app_tool_name(&tool_name)?;
    if validate_direct_system_read_arguments(&tool_name, &arguments)?
        && !direct_system_read_requires_approval(&tool_name)
    {
        return Ok(None);
    }
    ensure_trusted_builtin_mcp_server(&registry, &app, MACOS_APPLESCRIPT_SERVER_NAME).await?;
    registry
        .get_tool_details(MACOS_APPLESCRIPT_SERVER_NAME, &tool_name)
        .await
        .map_err(|error| error.message)?;
    registry
        .prepare_tool_approval(MACOS_APPLESCRIPT_SERVER_NAME, &tool_name, arguments)
        .await
        .map_err(|error| error.message)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn read_system_calendar(
    calendar_name: Option<String>,
    hours_ahead: Option<f64>,
    start_date: Option<String>,
    end_date: Option<String>,
    turn_context: Option<McpChatTurnContext>,
    registry: tauri::State<'_, McpClientRegistry>,
    persistence: tauri::State<'_, PersistenceEngine>,
    app: tauri::AppHandle,
) -> Result<McpToolCallResult, String> {
    let calendar_name = bounded_system_calendar_name(calendar_name);
    let hours_ahead = bounded_system_calendar_hours(hours_ahead);
    let start_date = bounded_system_calendar_datetime_text(start_date)?;
    let end_date = bounded_system_calendar_datetime_text(end_date)?;
    validate_system_calendar_window(start_date.as_deref(), end_date.as_deref())?;
    let arguments = serde_json::json!({
        "calendar_name": calendar_name,
        "hours_ahead": hours_ahead,
        "start_date": start_date,
        "end_date": end_date,
    });
    let receipt = native_apple_receipts::spec_for(
        MACOS_APPLESCRIPT_SERVER_NAME,
        READ_SYSTEM_CALENDAR_TOOL_NAME,
        &arguments,
    );
    let turn_context = turn_context.map(ChatTurnPersistenceContext::from);
    native_apple_receipts::execute(
        receipt,
        turn_context.as_ref(),
        persistence.inner(),
        true,
        read_system_calendar_with_deadline(
            calendar_name,
            hours_ahead,
            start_date,
            end_date,
            registry.inner(),
            &app,
        ),
    )
    .await
}

pub async fn execute_system_apple_app_tool(
    tool_name: String,
    arguments: Value,
    approval: Option<McpToolApproval>,
    turn_context: Option<McpChatTurnContext>,
    registry: tauri::State<'_, McpClientRegistry>,
    persistence: tauri::State<'_, PersistenceEngine>,
    app: tauri::AppHandle,
) -> Result<McpToolCallResult, String> {
    let tool_name = normalize_system_apple_app_tool_name(&tool_name)?;
    let turn_context = turn_context.map(ChatTurnPersistenceContext::from);
    let persistence = persistence.inner().clone();
    let guard = || validate_mcp_chat_turn(&persistence, turn_context.as_ref());
    if let Some(result) = native_capability_execution::execute_if_supported(
        MACOS_APPLESCRIPT_SERVER_NAME,
        &tool_name,
        &arguments,
        approval.clone(),
        turn_context.as_ref(),
        &persistence,
        registry.inner(),
        &app,
        &guard,
    )
    .await
    {
        return result;
    }
    if direct_system_read_display_name(&tool_name).is_some() {
        validate_tool_arguments(&arguments).map_err(|error| error.message)?;
        if direct_system_read_requires_approval(&tool_name) {
            return Err(format!(
                "{MCP_AUTHORIZATION_MESSAGE} Approval is required before reading {}.",
                direct_system_read_display_name(&tool_name).unwrap_or("this app")
            ));
        }
        guard().map_err(|error| error.message)?;
    }
    if tool_name == READ_SYSTEM_CALENDAR_TOOL_NAME {
        let (calendar_name, hours_ahead, start_date, end_date) =
            bounded_system_calendar_arguments(&arguments)?;
        return execute_direct_read(
            &tool_name,
            &arguments,
            turn_context.as_ref(),
            &persistence,
            read_system_calendar_with_deadline(
                calendar_name,
                hours_ahead,
                start_date,
                end_date,
                registry.inner(),
                &app,
            ),
        )
        .await;
    }
    if tool_name == READ_SYSTEM_PHOTOS_TOOL_NAME {
        let max_photos = crate::system_photos::photo_limit_from_arguments(&arguments)?;
        return execute_direct_read(
            &tool_name,
            &arguments,
            turn_context.as_ref(),
            &persistence,
            async move { Ok(crate::system_photos::read_system_photos_bounded(max_photos).await) },
        )
        .await;
    }
    if tool_name == READ_SYSTEM_CONTACTS_TOOL_NAME {
        let request = crate::tools::system_contacts::contact_request_from_arguments(&arguments)?;
        return execute_direct_read(
            &tool_name,
            &arguments,
            turn_context.as_ref(),
            &persistence,
            read_system_contacts_with_fallback(request, registry.inner(), &app),
        )
        .await;
    }
    if tool_name == READ_SYSTEM_MUSIC_TOOL_NAME {
        let max_songs = crate::system_music::song_limit_from_arguments(&arguments)?;
        return execute_direct_read(
            &tool_name,
            &arguments,
            turn_context.as_ref(),
            &persistence,
            async move { Ok(crate::system_music::read_system_music_bounded(max_songs).await) },
        )
        .await;
    }
    if tool_name == READ_SYSTEM_EMAILS_TOOL_NAME {
        let mail_arguments = arguments.clone();
        return execute_direct_read(
            &tool_name,
            &arguments,
            turn_context.as_ref(),
            &persistence,
            system_mail::execute_turn_bound_mail_read(
                mail_arguments,
                turn_context.as_ref(),
                registry.inner(),
                &persistence,
                &app,
            ),
        )
        .await;
    }
    ensure_trusted_builtin_mcp_server(&registry, &app, MACOS_APPLESCRIPT_SERVER_NAME).await?;
    registry
        .get_tool_details(MACOS_APPLESCRIPT_SERVER_NAME, &tool_name)
        .await
        .map_err(|error| error.message)?;
    let receipt =
        native_apple_receipts::spec_for(MACOS_APPLESCRIPT_SERVER_NAME, &tool_name, &arguments);
    let action_approved = approval.is_some();
    native_apple_receipts::execute(
        receipt,
        turn_context.as_ref(),
        &persistence,
        action_approved,
        async {
            registry
                .execute_tool_with_approval_guarded(
                    MACOS_APPLESCRIPT_SERVER_NAME,
                    &tool_name,
                    arguments,
                    approval,
                    &guard,
                )
                .await
                .map_err(|error| error.message)
        },
    )
    .await
}

async fn execute_direct_read<F>(
    tool_name: &str,
    arguments: &Value,
    turn: Option<&ChatTurnPersistenceContext>,
    persistence: &PersistenceEngine,
    future: F,
) -> Result<McpToolCallResult, String>
where
    F: std::future::Future<Output = Result<McpToolCallResult, String>>,
{
    let receipt =
        native_apple_receipts::spec_for(MACOS_APPLESCRIPT_SERVER_NAME, tool_name, arguments);
    native_apple_receipts::execute(receipt, turn, persistence, true, future).await
}
