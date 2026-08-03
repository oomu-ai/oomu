use rusqlite::Connection;
use std::path::Path;

fn query_conn(conn: &Connection) -> Result<(), Box<dyn std::error::Error>> {
    let table_count: i64 = conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='table'",
        [],
        |row| row.get(0),
    )?;
    println!("Tables count: {}", table_count);

    let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type='table'")?;
    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .filter_map(Result::ok)
        .collect();
    println!("Tables: {:?}", tables);

    if tables.contains(&"chat_sessions".to_string()) {
        println!("\n--- Chat Sessions ---");
        let mut stmt = conn.prepare("SELECT sql FROM sqlite_master WHERE name='chat_sessions'")?;
        let sql: String = stmt.query_row([], |row| row.get(0))?;
        println!("Schema: {}", sql);

        let mut stmt =
            conn.prepare("SELECT id, agent_id, title, provider_id, model_id FROM chat_sessions")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        for r in rows {
            let (id, agent_id, title, provider_id, model_id) = r?;
            println!(
                "Session ID: {} | Agent: {} | Title: {} | Provider: {} | Model: {}",
                id, agent_id, title, provider_id, model_id
            );
        }
    }

    if tables.contains(&"channel_configs".to_string()) {
        println!("\n--- Channel Configs ---");
        let mut stmt = conn.prepare(
            "SELECT platform, is_active, COALESCE(owner_id, ''), updated_at_ms FROM channel_configs ORDER BY platform",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        for r in rows {
            let (platform, is_active, owner_id, updated_at_ms) = r?;
            println!(
                "Platform: {} | Active: {} | Owner: {} | Updated: {}",
                platform,
                is_active == 1,
                if owner_id.is_empty() {
                    "not set"
                } else {
                    &owner_id
                },
                updated_at_ms
            );
        }
    }

    if tables.contains(&"chat_messages".to_string()) {
        println!("\n--- Last 20 chat messages ---");
        let mut stmt = conn.prepare(
            "SELECT id, session_id, role, content, timestamp_ms, COALESCE(provider_id, 'NULL'), COALESCE(model_id, 'NULL'), COALESCE(metadata_json, 'NULL') FROM chat_messages ORDER BY id DESC LIMIT 20"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
            ))
        })?;

        for r in rows {
            let (id, session_id, role, content, timestamp_ms, provider_id, model_id, metadata_json) =
                r?;
            println!("[ID: {}] Session: {} | Role: {} | Timestamp: {}\nProvider: {} | Model: {}\nMetadata: {}\nContent:\n{:?}\n------------------", id, session_id, role, timestamp_ms, provider_id, model_id, metadata_json, content);
        }
    } else if tables.contains(&"messages".to_string()) {
        println!("\n--- Last 20 messages from 'messages' table ---");
        let mut stmt =
            conn.prepare("SELECT id, role, content FROM messages ORDER BY id DESC LIMIT 20")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;

        for r in rows {
            let (id, role, content) = r?;
            println!(
                "[ID: {}] Role: {}\nContent:\n{:?}\n------------------",
                id, role, content
            );
        }
    } else {
        println!("No chat messages or messages table found.");
    }

    if tables.contains(&"chat_turns".to_string()) {
        println!("\n--- Recent Chat Turns (20) ---");
        let mut stmt = conn.prepare(
            "SELECT turn_id, generation_token, session_id, status, created_at_ms, COALESCE(completed_at_ms, 0) FROM chat_turns ORDER BY created_at_ms DESC LIMIT 20",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })?;
        for row in rows {
            let (turn_id, generation_token, session_id, status, created_at_ms, completed_at_ms) =
                row?;
            println!(
                "Turn: {turn_id} | Generation: {generation_token} | Session: {session_id} | Status: {status} | Created: {created_at_ms} | Completed: {completed_at_ms}"
            );
        }
    }

    if tables.contains(&"agent_configs".to_string()) {
        println!("\n--- Agent Configs Schema & Data ---");
        let mut stmt = conn.prepare("SELECT sql FROM sqlite_master WHERE name='agent_configs'")?;
        let sql: String = stmt.query_row([], |row| row.get(0))?;
        println!("Agent Configs Table Schema: {}", sql);

        let mut stmt =
            conn.prepare("SELECT id, name, status, COALESCE(model_id, 'NULL'), personality_profile FROM agent_configs")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        for r in rows {
            let (id, name, status, model_id, personality_profile) = r?;
            println!(
                "Agent: ID={}, Name={}, Status={}, ModelID={}\nPersonalityProfile: {}\n",
                id, name, status, model_id, personality_profile
            );
        }

        let mut stmt =
            conn.prepare("SELECT id, provider_id, provider_name, auth_method, base_url, auto_route_target FROM provider_configs")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })?;
        for r in rows {
            let (id, prov_id, name, auth, url, auto) = r?;
            println!(
                "ProviderConfig: ID={}, ProviderID={}, Name={}, Auth={}, BaseURL={}, AutoRoute={}",
                id,
                prov_id,
                name,
                auth,
                url,
                auto == 1
            );
        }
    }

    if tables.contains(&"agent_mods".to_string()) {
        println!("\n--- Agent Mod Bindings ---");
        let mut stmt =
            conn.prepare("SELECT agent_id, mod_id FROM agent_mods ORDER BY agent_id, mod_id")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for r in rows {
            let (agent_id, mod_id) = r?;
            println!("Agent ID: {} | Mod ID: {}", agent_id, mod_id);
        }
    }

    if tables.contains(&"installed_mods".to_string()) {
        println!("\n--- Installed Mods ---");

        let mut stmt = conn.prepare(
            "SELECT id, name, is_active, COALESCE(default_system_prompt, '') FROM installed_mods ORDER BY name COLLATE NOCASE"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        for r in rows {
            let (id, name, is_active, prompt) = r?;
            let prompt_preview = prompt.chars().take(120).collect::<String>();
            println!(
                "Mod ID: {} | Name: {} | Active: {} | Prompt: {:?}",
                id,
                name,
                is_active == 1,
                prompt_preview
            );
        }
    }

    if tables.contains(&"removed_built_in_mods".to_string()) {
        println!("\n--- Removed Built-in Mods ---");
        let mut stmt = conn.prepare("SELECT mod_id, removed_at_ms FROM removed_built_in_mods")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        for r in rows {
            let (mod_id, removed_at) = r?;
            println!("Removed Mod ID: {} | Removed At: {}", mod_id, removed_at);
        }
    }

    if tables.contains(&"agent_execution_logs".to_string()) {
        println!("\n--- Recent Agent Execution Logs (20) ---");
        let mut stmt = conn.prepare("SELECT id, level, phase, message, COALESCE(payload_json, 'NULL') FROM agent_execution_logs ORDER BY id DESC LIMIT 20")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        for r in rows {
            let (id, level, phase, message, payload) = r?;
            println!(
                "[ID: {}] Level: {} | Phase: {}\nMessage: {}\nPayload: {}\n------------------",
                id, level, phase, message, payload
            );
        }
    }
    Ok(())
}

fn try_open(path: &Path, key: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let conn = Connection::open(path)?;
    if let Some(k) = key {
        conn.pragma_update(None, "key", k)?;
    }
    query_conn(&conn)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = dirs::data_dir().ok_or("Could not find data directory")?;
    let paths = [
        data_dir
            .join("ai.eldris.oomu.gpd")
            .join("oomu_state.sqlite"),
        data_dir.join("ai.eldris.oomu.gpd").join("oomu_ops.db"),
        data_dir
            .join("ai.eldris.oomu.gpd")
            .join("release/pre_alpha/audit_024.sqlite"),
    ];

    let key = oomu_lib::db::get_database_key()?;
    println!("Obtained database key from native keychain.");

    for path in &paths {
        println!("\n=================================");
        println!("Checking path: {:?}", path);
        if !path.exists() {
            println!("File does not exist.");
            continue;
        }

        println!("-- Trying unencrypted...");
        match try_open(path, None) {
            Ok(_) => println!("SUCCESS without encryption!"),
            Err(e) => {
                println!("FAILED without encryption: {}", e);
                println!("-- Trying with derived key...");
                match try_open(path, Some(&key)) {
                    Ok(_) => println!("SUCCESS with encryption key!"),
                    Err(e2) => println!("FAILED with encryption key: {}", e2),
                }
            }
        }
    }

    Ok(())
}
