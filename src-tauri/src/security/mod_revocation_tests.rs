use super::{
    mod_trust,
    mods::{select_installed_mods, upsert_installed_mod, InstalledMod},
};
use crate::{db::PersistenceEngine, foundation::clock::unix_time_ms_i64};
use rusqlite::params;
use serde_json::json;
use std::fs;

#[test]
fn revoked_package_stays_withdrawn_after_reinstall_and_list() {
    let root = std::env::temp_dir().join(format!("oomu-sticky-revocation-{}", unix_time_ms_i64()));
    let installed = root.join("installed");
    fs::create_dir_all(&installed).unwrap();
    let manifest = json!({
        "id": "com.acme.revoked",
        "name": "Revoked package",
        "version": "1.0.0",
        "author": "Acme",
        "description": "Trust-state regression fixture.",
        "entrypoint": "index.js",
        "capability_bundle": {
            "id": "bundle_123e4567-e89b-12d3-a456-426614174000",
            "version": "1.0.0",
            "publisher": {"id": "acme", "name": "Acme"},
            "requestedGrants": []
        }
    });
    fs::write(installed.join("manifest.json"), manifest.to_string()).unwrap();
    fs::write(installed.join("index.js"), "export default true;").unwrap();
    let payload = mod_trust::evaluate_installed_directory(&installed)
        .unwrap()
        .trust
        .payload_sha256;
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let candidate = InstalledMod {
        id: "com.acme.revoked".into(),
        name: "Revoked package".into(),
        description: "Trust-state regression fixture.".into(),
        is_active: false,
        version: "1.0.0".into(),
        author: "Acme".into(),
        category: "Installed".into(),
        package_size: "1 KB".into(),
        last_updated: "Today".into(),
        review_state: "reviewed".into(),
        publisher_identity_verified: true,
        integrity_state: "verified".into(),
        is_built_in: false,
        permissions: Vec::new(),
        endpoints: Vec::new(),
        agent_config_schema: None,
        commands: None,
        requirements: None,
    };
    upsert_installed_mod(
        &engine, &candidate, &installed, &manifest, None, "index.js", &payload,
    )
    .unwrap();
    engine
        .open_connection()
        .unwrap()
        .execute(
            "INSERT INTO capability_registry_entries (bundle_id,package_version,catalog_revision,name,summary,category,publisher_name,review_state,compatibility_state,changelog,metadata_sha256,metadata_signature,cached_at_ms) VALUES (?1,'1.0.0','1','Revoked package','','','Acme','revoked','compatible','',?2,'test',1)",
            params!["bundle_123e4567-e89b-12d3-a456-426614174000", payload],
        )
        .unwrap();

    upsert_installed_mod(
        &engine, &candidate, &installed, &manifest, None, "index.js", &payload,
    )
    .unwrap();
    let persisted_review: String = engine
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT review_state FROM installed_mods WHERE id=?1",
            [&candidate.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(persisted_review, "revoked");
    let listed = select_installed_mods(&engine).unwrap();
    let package = listed.iter().find(|item| item.id == candidate.id).unwrap();
    assert_eq!(package.review_state, "revoked");
    assert!(!package.is_active);
}
