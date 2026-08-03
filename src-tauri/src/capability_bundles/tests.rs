use super::*;
use crate::{
    db::PersistenceEngine,
    projects::{CreateProjectRequest, ProjectDataPolicy},
};
use rusqlite::params;
use std::{fs, path::PathBuf};

fn setup() -> (PersistenceEngine, String, PathBuf) {
    let mut random = [0_u8; 8];
    rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut random);
    let root = std::env::temp_dir().join(format!(
        "oomu-bundle-test-{}-{}",
        crate::foundation::clock::unix_time_ms_i64(),
        hex::encode(random)
    ));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let project = crate::projects::repository::create(
        &engine,
        CreateProjectRequest {
            name: "Bundle".into(),
            description: String::new(),
            data_policy: ProjectDataPolicy::LocalOnly,
        },
    )
    .unwrap();
    let installed = root.join("installed-mod");
    fs::create_dir_all(&installed).unwrap();
    let connection = engine.open_connection().unwrap();
    connection.execute_batch("CREATE TABLE IF NOT EXISTS installed_mods (id TEXT PRIMARY KEY,name TEXT NOT NULL,description TEXT NOT NULL,is_active INTEGER NOT NULL DEFAULT 0,version TEXT NOT NULL,author TEXT NOT NULL,category TEXT NOT NULL,package_size TEXT NOT NULL,last_updated TEXT NOT NULL,permissions_json TEXT NOT NULL DEFAULT '[]',endpoints_json TEXT NOT NULL DEFAULT '[]',installed_path TEXT NOT NULL,manifest_json TEXT NOT NULL,default_system_prompt TEXT,entrypoint TEXT NOT NULL,installed_at_ms INTEGER NOT NULL,updated_at_ms INTEGER NOT NULL);").unwrap();
    let manifest = serde_json::json!({"id":"test.bundle","name":"Reports","version":"1.0.0","author":"Example","description":"Test","entrypoint":"main","permissions":{"allowed_paths":["Reports"]},"commands":[{"trigger":"/report"}]});
    fs::write(installed.join("manifest.json"), manifest.to_string()).unwrap();
    fs::write(installed.join("main"), "export default true;").unwrap();
    connection.execute("INSERT INTO installed_mods (id,name,description,is_active,version,author,category,package_size,last_updated,permissions_json,endpoints_json,installed_path,manifest_json,default_system_prompt,entrypoint,installed_at_ms,updated_at_ms) VALUES (?1,'Reports','Test',0,'1.0.0','Example','Work','1 KB','Today','[]','[]',?2,?3,NULL,'main',1,1)",params!["test.bundle",installed.to_string_lossy(),manifest.to_string()]).unwrap();
    (engine, project.project_id, installed)
}
#[test]
fn legacy_mod_uses_one_adapter_and_requires_unreviewed_ack() {
    let (engine, project, _) = setup();
    let record = repository::inspect(
        &engine,
        InspectBundleRequest {
            mod_id: "test.bundle".into(),
            project_ids: vec![project.clone()],
        },
    )
    .unwrap();
    assert_eq!(record.review_state, "unreviewed");
    assert!(repository::activate(
        &engine,
        ActivateBundleRequest {
            bundle_id: record.bundle_id.clone(),
            package_version: record.package_version.clone(),
            acknowledge_unreviewed: false
        }
    )
    .is_err());
    let active = repository::activate(
        &engine,
        ActivateBundleRequest {
            bundle_id: record.bundle_id.clone(),
            package_version: record.package_version,
            acknowledge_unreviewed: true,
        },
    )
    .unwrap();
    assert_eq!(active.review_state, "unreviewed");
    assert_eq!(active.integrity_state, "unsigned");
    assert!(repository::authorize(
        &engine,
        BundleAuthorityRequest {
            bundle_id: record.bundle_id,
            project_id: project,
            capability: "file".into(),
            requested_scope: "Reports".into()
        }
    )
    .is_ok());
}

#[test]
fn global_mod_activation_does_not_require_a_project_assignment() {
    let (engine, project, _) = setup();
    let record = repository::inspect(
        &engine,
        InspectBundleRequest {
            mod_id: "test.bundle".into(),
            project_ids: Vec::new(),
        },
    )
    .unwrap();

    let active = repository::activate(
        &engine,
        ActivateBundleRequest {
            bundle_id: record.bundle_id.clone(),
            package_version: record.package_version,
            acknowledge_unreviewed: true,
        },
    )
    .unwrap();

    assert!(active.project_ids.is_empty());
    assert_eq!(active.install_state, "active");
    assert!(repository::authorize(
        &engine,
        BundleAuthorityRequest {
            bundle_id: record.bundle_id,
            project_id: project,
            capability: "file".into(),
            requested_scope: "Reports".into(),
        }
    )
    .is_ok());
}

#[test]
fn undeclared_capability_fails_closed() {
    let (engine, project, _) = setup();
    let record = repository::inspect(
        &engine,
        InspectBundleRequest {
            mod_id: "test.bundle".into(),
            project_ids: vec![project.clone()],
        },
    )
    .unwrap();
    repository::activate(
        &engine,
        ActivateBundleRequest {
            bundle_id: record.bundle_id.clone(),
            package_version: record.package_version,
            acknowledge_unreviewed: true,
        },
    )
    .unwrap();
    assert!(repository::authorize(
        &engine,
        BundleAuthorityRequest {
            bundle_id: record.bundle_id,
            project_id: project,
            capability: "network".into(),
            requested_scope: "*".into()
        }
    )
    .is_err());
}

#[test]
fn review_state_survives_activation_and_disable() {
    let (engine, project, _) = setup();
    let record = repository::inspect(
        &engine,
        InspectBundleRequest {
            mod_id: "test.bundle".into(),
            project_ids: vec![project],
        },
    )
    .unwrap();
    let active = repository::activate(
        &engine,
        ActivateBundleRequest {
            bundle_id: record.bundle_id.clone(),
            package_version: record.package_version.clone(),
            acknowledge_unreviewed: true,
        },
    )
    .unwrap();
    let disabled = repository::disable(
        &engine,
        BundleVersionRequest {
            bundle_id: record.bundle_id,
            package_version: record.package_version,
        },
    )
    .unwrap();
    assert_eq!(active.review_state, "unreviewed");
    assert_eq!(disabled.review_state, "unreviewed");
    assert_eq!(disabled.install_state, "disabled");
}

#[test]
fn revoked_state_cannot_be_acknowledged_away() {
    let (engine, project, _) = setup();
    let record = repository::inspect(
        &engine,
        InspectBundleRequest {
            mod_id: "test.bundle".into(),
            project_ids: vec![project],
        },
    )
    .unwrap();
    engine.open_connection().unwrap().execute(
        "UPDATE capability_bundle_records SET review_state='revoked' WHERE bundle_id=?1 AND package_version=?2",
        params![record.bundle_id, record.package_version],
    ).unwrap();
    assert!(repository::activate(
        &engine,
        ActivateBundleRequest {
            bundle_id: record.bundle_id,
            package_version: record.package_version,
            acknowledge_unreviewed: true,
        },
    )
    .is_err());
}

#[test]
fn changed_files_are_quarantined_before_activation() {
    let (engine, project, installed) = setup();
    let record = repository::inspect(
        &engine,
        InspectBundleRequest {
            mod_id: "test.bundle".into(),
            project_ids: vec![project],
        },
    )
    .unwrap();
    fs::write(installed.join("main"), "export default false;").unwrap();
    assert!(repository::activate(
        &engine,
        ActivateBundleRequest {
            bundle_id: record.bundle_id,
            package_version: record.package_version,
            acknowledge_unreviewed: true,
        },
    )
    .is_err());
    let integrity: String = engine
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT integrity_state FROM installed_mods WHERE id='test.bundle'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(integrity, "modified");
}

#[test]
fn canonical_p1_bundle_contract_uses_authoritative_fields() {
    let digest = "00".repeat(32);
    let signature = "00".repeat(64);
    let value = serde_json::json!({
        "schemaVersion": 1,
        "contractType": "capability_bundle",
        "capabilityBundleId": "bundle_123e4567-e89b-12d3-a456-426614174000",
        "name": "Reports",
        "packageVersion": "1.0.0",
        "publisher": {"id": "acme", "name": "Acme"},
        "scope": {"kind": "global", "projectIds": []},
        "capabilities": ["file"],
        "requestedGrants": [{"capability": "file", "scope": "Reports", "reason": "Creates reports."}],
        "payloadSha256": digest.clone(),
        "evidence": [],
        "signature": {
            "algorithm": "ed25519", "keyId": "acme-v1", "payloadSha256": digest,
            "signature": signature, "signedAt": "2026-07-13T00:00:00Z"
        }
    });
    let (id, version, publisher_id, publisher_name, grants) =
        repository::bundle_identity(&value).unwrap();
    assert_eq!(id, "bundle_123e4567-e89b-12d3-a456-426614174000");
    assert_eq!(version, "1.0.0");
    assert_eq!(
        (publisher_id.as_str(), publisher_name.as_str()),
        ("acme", "Acme")
    );
    assert_eq!(grants[0].capability, "file");
}
