use super::*;

pub async fn mcp_execute_tool(
    server_name: String,
    tool_name: String,
    arguments: Value,
    approval: Option<McpToolApproval>,
    turn_context: Option<McpChatTurnContext>,
    registry: tauri::State<'_, McpClientRegistry>,
    persistence: tauri::State<'_, PersistenceEngine>,
    app: tauri::AppHandle,
) -> Result<McpToolCallResult, String> {
    let turn_context = turn_context.map(ChatTurnPersistenceContext::from);
    let persistence = persistence.inner().clone();
    let guard = || validate_mcp_chat_turn(&persistence, turn_context.as_ref());
    if let Some(result) = native_public_search_execution::execute_if_supported(
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
    if is_system_calendar_tool(&server_name, &tool_name) {
        validate_tool_arguments(&arguments).map_err(|error| error.message)?;
        guard().map_err(|error| error.message)?;
        let (calendar_name, hours_ahead, start_date, end_date) =
            bounded_system_calendar_arguments(&arguments)?;
        let receipt = native_apple_receipts::spec_for(&server_name, &tool_name, &arguments);
        return native_apple_receipts::execute(
            receipt,
            turn_context.as_ref(),
            &persistence,
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
        .await;
    }
    if is_system_contacts_tool(&server_name, &tool_name) {
        validate_tool_arguments(&arguments).map_err(|error| error.message)?;
        guard().map_err(|error| error.message)?;
        let request = crate::tools::system_contacts::contact_request_from_arguments(&arguments)?;
        let receipt = native_apple_receipts::spec_for(&server_name, &tool_name, &arguments);
        return native_apple_receipts::execute(
            receipt,
            turn_context.as_ref(),
            &persistence,
            true,
            read_system_contacts_with_fallback(request, registry.inner(), &app),
        )
        .await;
    }
    if is_system_photos_tool(&server_name, &tool_name) {
        validate_tool_arguments(&arguments).map_err(|error| error.message)?;
        guard().map_err(|error| error.message)?;
        let max_photos = crate::system_photos::photo_limit_from_arguments(&arguments)?;
        let receipt = native_apple_receipts::spec_for(&server_name, &tool_name, &arguments);
        return native_apple_receipts::execute(
            receipt,
            turn_context.as_ref(),
            &persistence,
            true,
            async move { Ok(crate::system_photos::read_system_photos_bounded(max_photos).await) },
        )
        .await;
    }
    if is_system_music_tool(&server_name, &tool_name) {
        validate_tool_arguments(&arguments).map_err(|error| error.message)?;
        guard().map_err(|error| error.message)?;
        let max_songs = crate::system_music::song_limit_from_arguments(&arguments)?;
        let receipt = native_apple_receipts::spec_for(&server_name, &tool_name, &arguments);
        return native_apple_receipts::execute(
            receipt,
            turn_context.as_ref(),
            &persistence,
            true,
            async move { Ok(crate::system_music::read_system_music_bounded(max_songs).await) },
        )
        .await;
    }
    if system_mail::is_system_mail_read_tool(&server_name, &tool_name) {
        let receipt = native_apple_receipts::spec_for(&server_name, &tool_name, &arguments);
        return native_apple_receipts::execute(
            receipt,
            turn_context.as_ref(),
            &persistence,
            true,
            system_mail::execute_turn_bound_mail_read(
                arguments,
                turn_context.as_ref(),
                registry.inner(),
                &persistence,
                &app,
            ),
        )
        .await;
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
