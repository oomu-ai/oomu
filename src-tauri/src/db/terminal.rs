pub(super) fn terminal_optional_bool(value: Option<i64>) -> &'static str {
    match value {
        Some(0) => "false",
        Some(_) => "true",
        None => "default",
    }
}

pub(super) fn terminal_empty_placeholder(value: &str) -> &str {
    if value.trim().is_empty() {
        "default"
    } else {
        value
    }
}

pub(super) fn terminal_preview(value: &str, max_chars: usize) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_chars {
        return compact;
    }
    let mut preview = compact.chars().take(max_chars).collect::<String>();
    preview.push_str("...");
    preview
}
use super::*;

pub fn execute_terminal_db_audit() -> Result<(), String> {
    let db_path = project_root().join(DB_FILE);
    println!("State database: {}", db_path.display());
    if !db_path.exists() {
        println!("State database was not found. Nothing to dump.");
        return Ok(());
    }
    let database_key = get_database_key()?;
    let connection = open_sqlcipher_database_connection_with_key(&db_path, &database_key)
        .map_err(|error| error.to_string())?;
    let tables: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    println!("Tables: {tables}");
    dump_recent_chat_sessions(&connection).map_err(|error| error.to_string())?;
    dump_active_configurations(&connection).map_err(|error| error.to_string())?;
    dump_recent_agent_executions(&connection).map_err(|error| error.to_string())?;
    dump_recent_auto_route_evidence(&database_key).map_err(|error| error.to_string())?;
    dump_installed_mods(&connection).map_err(|error| error.to_string())
}

fn dump_recent_agent_executions(connection: &Connection) -> rusqlite::Result<()> {
    println!("\n--- Recent Agent Executions (10) ---");
    if !table_exists(connection, "agent_executions")?
        || !table_exists(connection, "agent_execution_logs")?
    {
        println!("Agent execution history is not available.");
        return Ok(());
    }

    let mut executions = connection.prepare(
        "SELECT execution_id, plan_id, session_id, provider_id, model_id, status,
                json_extract(context_json, '$.turn_context.automatedWebGroundingEnabled'),
                created_at_ms, updated_at_ms
         FROM agent_executions
         ORDER BY updated_at_ms DESC
         LIMIT 10",
    )?;
    let rows = executions.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, Option<i64>>(6)?,
            row.get::<_, i64>(7)?,
            row.get::<_, i64>(8)?,
        ))
    })?;
    let mut count = 0;
    for row in rows {
        let (
            execution_id,
            plan_id,
            session_id,
            provider_id,
            model_id,
            status,
            web_grounding,
            created_at_ms,
            updated_at_ms,
        ) = row?;
        count += 1;
        println!(
            "#{count} execution={} plan={} session={} route={}/{} web_grounding={} status={} created_at_ms={} updated_at_ms={}",
            execution_id,
            plan_id,
            session_id,
            provider_id,
            model_id,
            terminal_optional_bool(web_grounding),
            status,
            created_at_ms,
            updated_at_ms
        );
        let mut logs = connection.prepare(
            "SELECT level, phase, message, COALESCE(payload_json, ''), created_at_ms
             FROM agent_execution_logs
             WHERE execution_id=?1
             ORDER BY id DESC
             LIMIT 8",
        )?;
        let log_rows = logs.query_map(params![execution_id], |log| {
            Ok((
                log.get::<_, String>(0)?,
                log.get::<_, String>(1)?,
                log.get::<_, String>(2)?,
                log.get::<_, String>(3)?,
                log.get::<_, i64>(4)?,
            ))
        })?;
        for log in log_rows {
            let (level, phase, message, payload, timestamp) = log?;
            println!(
                "  log level={} phase={} at_ms={} message={} payload={}",
                level,
                phase,
                timestamp,
                terminal_preview(&message, 240),
                terminal_preview(&payload, 240)
            );
        }
        let mut actions = connection.prepare(
            "SELECT id, tool, status, COALESCE(output, ''), timestamp_ms
             FROM actions
             WHERE plan_id=?1
             ORDER BY id ASC",
        )?;
        let action_rows = actions.query_map(params![plan_id], |action| {
            Ok((
                action.get::<_, i64>(0)?,
                action.get::<_, String>(1)?,
                action.get::<_, String>(2)?,
                action.get::<_, String>(3)?,
                action.get::<_, i64>(4)?,
            ))
        })?;
        for action in action_rows {
            let (id, tool, action_status, output, timestamp) = action?;
            println!(
                "  action id={} tool={} status={} at_ms={} output={}",
                id,
                tool,
                action_status,
                timestamp,
                terminal_preview(&output, 280)
            );
        }
        let generation_state = connection
            .query_row(
                "SELECT current_step_index, status, generated_text, timestamp_ms
                 FROM plan_generation_states WHERE plan_id=?1",
                params![plan_id],
                |state| {
                    Ok((
                        state.get::<_, i64>(0)?,
                        state.get::<_, String>(1)?,
                        state.get::<_, String>(2)?,
                        state.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()?;
        if let Some((step, plan_status, text, timestamp)) = generation_state {
            println!(
                "  checkpoint step={} status={} at_ms={} detail={}",
                step,
                plan_status,
                timestamp,
                terminal_preview(&text, 280)
            );
        }
        let recovery_receipt = connection
            .query_row(
                "SELECT json_extract(content, '$.code'),
                        json_extract(content, '$.boundary'),
                        json_extract(content, '$.recoveryAction'),
                        json_extract(content, '$.changedState'),
                        json_extract(content, '$.message'), timestamp_ms
                 FROM chat_messages
                 WHERE session_id=?1 AND role='assistant' AND json_valid(content)
                   AND json_extract(content, '$.schema')=?2
                   AND json_extract(content, '$.executionId')=?3
                 ORDER BY id DESC LIMIT 1",
                params![
                    session_id,
                    crate::agentic_loop::recovery::RECOVERY_RECEIPT_SCHEMA,
                    execution_id
                ],
                |receipt| {
                    Ok((
                        receipt.get::<_, String>(0)?,
                        receipt.get::<_, String>(1)?,
                        receipt.get::<_, Option<String>>(2)?,
                        receipt.get::<_, String>(3)?,
                        receipt.get::<_, String>(4)?,
                        receipt.get::<_, i64>(5)?,
                    ))
                },
            )
            .optional()?;
        if let Some((code, boundary, recovery_action, changed_state, message, timestamp)) =
            recovery_receipt
        {
            println!(
                "  recovery code={} boundary={} action={} changed_state={} at_ms={} message={}",
                code,
                boundary,
                recovery_action.as_deref().unwrap_or("legacy"),
                changed_state,
                timestamp,
                terminal_preview(&message, 280)
            );
        }
        let task_state = connection
            .query_row(
                "SELECT state, COALESCE(last_error, ''), recovery_state, updated_at_ms
                 FROM task_runs
                 WHERE runtime_kind='agent' AND runtime_record_id=?1
                 ORDER BY updated_at_ms DESC LIMIT 1",
                params![execution_id],
                |task| {
                    Ok((
                        task.get::<_, String>(0)?,
                        task.get::<_, String>(1)?,
                        task.get::<_, String>(2)?,
                        task.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()?;
        if let Some((state, last_error, recovery_state, timestamp)) = task_state {
            println!(
                "  task state={} recovery={} at_ms={} error={}",
                state,
                recovery_state,
                timestamp,
                terminal_preview(&last_error, 320)
            );
        }
    }
    if count == 0 {
        println!("No agent executions found.");
    }
    Ok(())
}

fn dump_recent_auto_route_evidence(database_key: &str) -> rusqlite::Result<()> {
    println!("\n--- Recent Auto-route Evidence (20) ---");
    let ops_path = project_root().join(OPS_DB_FILE);
    if !ops_path.exists() {
        println!("No Auto-route evidence database found.");
        return Ok(());
    }
    let connection = open_ops_database_connection_with_key(&ops_path, database_key)?;
    if !table_exists(&connection, "local_inference_audit")? {
        println!("No Auto-route evidence table found.");
        return Ok(());
    }
    let mut statement = connection.prepare(
        "SELECT event_kind, metadata_json, created_at_ms
         FROM local_inference_audit
         WHERE event_kind IN ('dynamic_routing', 'auto_route_classifier_health')
         ORDER BY created_at_ms DESC LIMIT 20",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    let mut count = 0;
    for row in rows {
        let (event_kind, metadata_json, created_at_ms) = row?;
        count += 1;
        println!(
            "#{count} kind={} created_at_ms={} evidence={}",
            event_kind,
            created_at_ms,
            terminal_preview(&metadata_json, 360),
        );
    }
    if count == 0 {
        println!("No Auto-route decisions or readiness failures found.");
    }
    Ok(())
}
