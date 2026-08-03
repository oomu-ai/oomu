use super::*;
use serde_json::json;

fn test_temp_dir(name: &str) -> PathBuf {
    let test_dir = std::env::temp_dir().join(format!(
        "oomu_mods_{name}_{}_{}",
        std::process::id(),
        now_ms()
    ));
    fs::create_dir_all(&test_dir).expect("test directory created");
    test_dir
}

fn test_engine(name: &str) -> PersistenceEngine {
    let test_dir = test_temp_dir(name);
    let db_path = test_dir.join("state.sqlite");
    PersistenceEngine::initialize_at(db_path).expect("test persistence initializes")
}

fn manifest_json_with_permissions(permissions: Value) -> Value {
    json!({
        "id": "test.mod.cs",
        "name": "Test CS Mod",
        "version": "1.0.0",
        "author": "OOMU",
        "description": "Test description",
        "category": "Customer Service",
        "default_system_prompt": null,
        "permissions": permissions,
        "entrypoint": "index.js"
    })
}

fn insert_test_mod(
    connection: &rusqlite::Connection,
    id: &str,
    name: &str,
    active: bool,
    prompt: Option<&str>,
) {
    let installed = test_temp_dir(&format!("fixture_{}", storage_id(id)));
    let manifest = json!({
        "id": id, "name": name, "version": "1.0.0", "author": "Test",
        "description": "Test mod", "entrypoint": "index.js",
        "default_system_prompt": prompt
    });
    fs::write(installed.join("index.js"), "export default true;").unwrap();
    fs::write(installed.join("manifest.json"), manifest.to_string()).unwrap();
    connection
        .execute(
            "
                INSERT INTO installed_mods (
                    id, name, description, is_active, version, author, category,
                    package_size, last_updated, permissions_json, endpoints_json,
                    installed_path, manifest_json, default_system_prompt, entrypoint,
                    installed_at_ms, updated_at_ms
                )
                VALUES (?1, ?2, 'Test mod', ?3, '1.0.0', 'Test', 'Prompt Hook',
                        '1 KB', 'June 22, 2026', '[]', '[]', ?4,
                        ?5, ?6, 'index.js', ?7, ?7)
                ",
            params![
                id,
                name,
                bool_to_db(active),
                installed.to_string_lossy(),
                manifest.to_string(),
                prompt,
                now_ms()
            ],
        )
        .expect("test mod inserted");
}

mod compatibility;
mod install;
mod package;
mod permission;
