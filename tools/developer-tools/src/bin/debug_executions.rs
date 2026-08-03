use oomu_lib::db::get_database_key;
use rusqlite::Connection;
use zeroize::Zeroize;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = dirs::data_dir().ok_or("Could not find data directory")?;
    let path = data_dir
        .join("ai.eldris.oomu.gpd")
        .join("oomu_state.sqlite");
    if !path.exists() {
        println!("Database file does not exist at {:?}", path);
        return Ok(());
    }

    let mut key = get_database_key().map_err(|message| {
        std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Database key unavailable: {message}"),
        )
    })?;
    println!("Using process-scoped database key resolver...");

    let conn = Connection::open(&path)?;
    conn.pragma_update(None, "key", &key)?;
    key.zeroize();

    let count: i64 = conn.query_row("SELECT COUNT(*) FROM execution_instances", [], |row| {
        row.get(0)
    })?;
    println!("Total execution_instances rows: {}", count);

    let wf_count: i64 = conn.query_row("SELECT COUNT(*) FROM workflows", [], |row| row.get(0))?;
    println!("Total workflows rows: {}", wf_count);

    let b_count: i64 = conn.query_row("SELECT COUNT(*) FROM workflow_blueprints", [], |row| {
        row.get(0)
    })?;
    println!("Total workflow_blueprints rows: {}", b_count);

    if count > 0 {
        println!("\n--- Fetching last 5 executions ---");
        let mut stmt = conn.prepare(
            "SELECT id, workflow_id, status, created_at_ms, node_payloads_json, memory_json, error_json FROM execution_instances ORDER BY created_at_ms DESC LIMIT 5"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        })?;
        for r in rows {
            let (id, wf_id, status, created, payloads, memory, error) = r?;
            println!("==================================================");
            println!(
                "ID: {} | WF_ID: {} | Status: {} | Created: {}",
                id, wf_id, status, created
            );
            println!("Error: {:?}", error);
            println!("Node Payloads (JSON):");
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&payloads) {
                println!("{}", serde_json::to_string_pretty(&val)?);
            } else {
                println!("{}", payloads);
            }
            println!("Memory (JSON):");
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&memory) {
                println!("{}", serde_json::to_string_pretty(&val)?);
            } else {
                println!("{}", memory);
            }
        }
    }

    Ok(())
}
