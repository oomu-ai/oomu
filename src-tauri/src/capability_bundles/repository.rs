use super::*;
use crate::{db::PersistenceEngine, p0_contracts::ProjectId};
use rand_core::{OsRng, RngCore};
use rusqlite::{params, OptionalExtension, Row};

fn random_id(prefix: &str) -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    format!("{prefix}_{}", hex::encode(bytes))
}
fn valid_version(value: &str) -> bool {
    let base = value.split_once('-').map(|v| v.0).unwrap_or(value);
    let parts = base.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts
            .iter()
            .all(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
}
fn clean_grants(value: &serde_json::Value) -> Result<Vec<CapabilityGrant>, String> {
    let raw = value
        .get("requestedGrants")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if raw.len() > 256 {
        return Err("This bundle asks for too many abilities.".into());
    }
    raw.into_iter()
        .map(|item| {
            let capability = item
                .get("capability")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let bounded_scope = item
                .get("scope")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            let reason = item
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if !CAPABILITY_KINDS.contains(&capability)
                || bounded_scope.is_empty()
                || bounded_scope.len() > 512
                || reason.is_empty()
                || reason.len() > 2_000
            {
                return Err("This bundle has an incomplete ability description.".into());
            }
            Ok(CapabilityGrant {
                capability: capability.into(),
                bounded_scope: bounded_scope.into(),
                reason: reason.into(),
            })
        })
        .collect()
}

fn capability_name(value: crate::p1_contracts::CapabilityKind) -> &'static str {
    use crate::p1_contracts::CapabilityKind;
    match value {
        CapabilityKind::File => "file",
        CapabilityKind::Network => "network",
        CapabilityKind::Connector => "connector",
        CapabilityKind::Model => "model",
        CapabilityKind::Executable => "executable",
        CapabilityKind::Schedule => "schedule",
        CapabilityKind::ChildAgent => "child_agent",
        CapabilityKind::Mutation => "mutation",
    }
}

pub(super) fn bundle_identity(
    value: &serde_json::Value,
) -> Result<(String, String, String, String, Vec<CapabilityGrant>), String> {
    if value.get("schemaVersion").is_some() || value.get("contractType").is_some() {
        let bundle: crate::p1_contracts::CapabilityBundle =
            serde_json::from_value(value.clone())
                .map_err(|error| format!("The capability bundle contract is invalid: {error}"))?;
        let grants = bundle
            .requested_grants
            .into_iter()
            .map(|grant| CapabilityGrant {
                capability: capability_name(grant.capability).to_string(),
                bounded_scope: grant.scope,
                reason: grant.reason,
            })
            .collect();
        return Ok((
            bundle.capability_bundle_id.to_string(),
            bundle.package_version,
            bundle.publisher.id,
            bundle.publisher.name,
            grants,
        ));
    }
    let id = value.get("id").and_then(|item| item.as_str()).unwrap_or("");
    crate::p1_contracts::CapabilityBundleId::parse(id)?;
    let version = value
        .get("version")
        .and_then(|item| item.as_str())
        .unwrap_or("");
    if !valid_version(version) {
        return Err("The bundle version is invalid.".into());
    }
    Ok((
        id.into(),
        version.into(),
        value
            .pointer("/publisher/id")
            .and_then(|item| item.as_str())
            .unwrap_or("")
            .into(),
        value
            .pointer("/publisher/name")
            .and_then(|item| item.as_str())
            .unwrap_or("")
            .into(),
        clean_grants(value)?,
    ))
}
fn legacy_grants(manifest: &serde_json::Value) -> Vec<CapabilityGrant> {
    let mut grants = Vec::new();
    if let Some(paths) = manifest
        .pointer("/permissions/allowed_paths")
        .and_then(|v| v.as_array())
    {
        for path in paths.iter().filter_map(|v| v.as_str()) {
            grants.push(CapabilityGrant {
                capability: "file".into(),
                bounded_scope: path.into(),
                reason: "Uses files you explicitly make available.".into(),
            });
        }
    }
    if let Some(hosts) = manifest
        .pointer("/permissions/allowed_hosts")
        .and_then(|v| v.as_array())
    {
        for host in hosts.iter().filter_map(|v| v.as_str()) {
            grants.push(CapabilityGrant {
                capability: "network".into(),
                bounded_scope: host.into(),
                reason: "Connects to this service when needed.".into(),
            });
        }
    }
    if manifest
        .get("commands")
        .and_then(|v| v.as_array())
        .is_some_and(|v| !v.is_empty())
    {
        grants.push(CapabilityGrant {
            capability: "mutation".into(),
            bounded_scope: "agent_conversation".into(),
            reason: "Adds actions you can choose in a conversation.".into(),
        });
    }
    grants
}
fn receipt(
    connection: &rusqlite::Connection,
    bundle: &str,
    version: &str,
    event: &str,
    detail: serde_json::Value,
) -> Result<(), String> {
    connection.execute("INSERT INTO capability_bundle_receipts (receipt_id,bundle_id,package_version,event_kind,detail_json,created_at_ms) VALUES (?1,?2,?3,?4,?5,?6)", params![random_id("receipt"),bundle,version,event,detail.to_string(),crate::foundation::clock::unix_time_ms_i64()]).map(|_|()).map_err(|e|e.to_string())
}
pub fn inspect(
    engine: &PersistenceEngine,
    request: InspectBundleRequest,
) -> Result<CapabilityBundleRecord, String> {
    engine.require_durable_store("inspect a capability bundle")?;
    let _operation_guard = crate::security::mod_trust::lock_mod_package_operation()?;
    let trust = crate::security::mods::reverify_installed_mod_trust(engine, &request.mod_id)?;
    let connection = engine.open_connection().map_err(|e| e.to_string())?;
    let raw: Option<(String,String,String,String)>=connection.query_row("SELECT manifest_json,name,version,author FROM installed_mods WHERE id=?1 AND is_active=0",params![request.mod_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional().map_err(|e|e.to_string())?;
    let Some((manifest_json, _name, legacy_version, legacy_author)) = raw else {
        return Err("Install this bundle before reviewing it.".into());
    };
    let manifest: serde_json::Value = serde_json::from_str(&manifest_json)
        .map_err(|_| "The installed package record is invalid.".to_string())?;
    let (bundle_id, version, publisher_id, publisher_name, grants): (
        String,
        String,
        String,
        String,
        Vec<CapabilityGrant>,
    ) = if let Some(bundle) = manifest.get("capability_bundle") {
        bundle_identity(bundle)?
    } else {
        let digest = crate::foundation::digest::sha256_hex(manifest_json.as_bytes());
        (
            format!("bundle_legacy_{}", &digest[..32]),
            legacy_version,
            "legacy".into(),
            legacy_author,
            legacy_grants(&manifest),
        )
    };
    if version != trust.version {
        return Err("The installed mod version changed before review.".to_string());
    }
    let identity_verified = trust.publisher_identity_verified;
    let payload_sha = trust.payload_sha256;
    for id in &request.project_ids {
        ProjectId::parse(id)?;
        let exists = connection
            .query_row(
                "SELECT 1 FROM projects WHERE project_id=?1",
                params![id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .is_some();
        if !exists {
            return Err("A selected Project is unavailable.".into());
        }
    }
    let revoked: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM capability_bundle_records WHERE bundle_id=?1 AND package_version=?2 AND review_state='revoked' UNION ALL SELECT 1 FROM capability_registry_entries WHERE bundle_id=?1 AND package_version=?2 AND metadata_sha256=?3 AND review_state='revoked')",params![bundle_id,version,payload_sha],|row|row.get(0)).map_err(|e|e.to_string())?;
    let review = if revoked {
        "revoked"
    } else {
        trust.review_state.as_str()
    };
    let previous: Option<String>=connection.query_row("SELECT package_version FROM capability_bundle_records WHERE bundle_id=?1 AND install_state IN ('active','disabled') ORDER BY installed_at_ms DESC LIMIT 1",params![bundle_id],|row|row.get(0)).optional().map_err(|e|e.to_string())?;
    let now = crate::foundation::clock::unix_time_ms_i64();
    connection.execute("INSERT INTO capability_bundle_records (bundle_id,package_version,mod_id,publisher_id,publisher_name,publisher_identity_verified,review_state,compatibility_state,payload_sha256,manifest_json,capabilities_json,project_ids_json,install_state,previous_version,updated_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,'compatible',?8,?9,?10,?11,'inspected',?12,?13) ON CONFLICT(bundle_id,package_version) DO UPDATE SET publisher_identity_verified=excluded.publisher_identity_verified,review_state=excluded.review_state,payload_sha256=excluded.payload_sha256,manifest_json=excluded.manifest_json,capabilities_json=excluded.capabilities_json,project_ids_json=excluded.project_ids_json,install_state='inspected',previous_version=excluded.previous_version,updated_at_ms=excluded.updated_at_ms",params![bundle_id,version,request.mod_id,publisher_id,publisher_name,identity_verified,review,payload_sha,manifest_json,serde_json::to_string(&grants).map_err(|e|e.to_string())?,serde_json::to_string(&request.project_ids).map_err(|e|e.to_string())?,previous,now]).map_err(|e|e.to_string())?;
    receipt(
        &connection,
        &bundle_id,
        &version,
        "inspected",
        serde_json::json!({"reviewState":review,"publisherIdentityVerified":identity_verified,"capabilityCount":grants.len()}),
    )?;
    get_unlocked(engine, &bundle_id, &version)
}
fn from_row(row: &Row<'_>) -> rusqlite::Result<CapabilityBundleRecord> {
    let grants: String = row.get(8)?;
    let projects: String = row.get(9)?;
    Ok(CapabilityBundleRecord {
        bundle_id: row.get(0)?,
        package_version: row.get(1)?,
        mod_id: row.get(2)?,
        name: row.get(3)?,
        publisher_name: row.get(4)?,
        publisher_identity_verified: row.get::<_, i64>(5)? != 0,
        review_state: row.get(6)?,
        integrity_state: row.get(7)?,
        capabilities: serde_json::from_str(&grants).unwrap_or_default(),
        project_ids: serde_json::from_str(&projects).unwrap_or_default(),
        compatibility_state: row.get(10)?,
        install_state: row.get(11)?,
        previous_version: row.get(12)?,
        updated_at_ms: row.get(13)?,
    })
}
const SELECT:&str="SELECT b.bundle_id,b.package_version,b.mod_id,m.name,b.publisher_name,b.publisher_identity_verified,b.review_state,m.integrity_state,b.capabilities_json,b.project_ids_json,b.compatibility_state,b.install_state,b.previous_version,b.updated_at_ms FROM capability_bundle_records b JOIN installed_mods m ON m.id=b.mod_id";
fn get_unlocked(
    engine: &PersistenceEngine,
    bundle: &str,
    version: &str,
) -> Result<CapabilityBundleRecord, String> {
    engine
        .open_connection()
        .map_err(|e| e.to_string())?
        .query_row(
            &format!("{SELECT} WHERE b.bundle_id=?1 AND b.package_version=?2"),
            params![bundle, version],
            from_row,
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Capability bundle was not found.".into())
}
pub fn list(engine: &PersistenceEngine) -> Result<Vec<CapabilityBundleRecord>, String> {
    let connection = engine.open_connection().map_err(|e| e.to_string())?;
    let mut statement = connection
        .prepare(&format!("{SELECT} ORDER BY b.updated_at_ms DESC"))
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([], from_row)
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}
pub fn activate(
    engine: &PersistenceEngine,
    request: ActivateBundleRequest,
) -> Result<CapabilityBundleRecord, String> {
    let _operation_guard = crate::security::mod_trust::lock_mod_package_operation()?;
    let mut record = get_unlocked(engine, &request.bundle_id, &request.package_version)?;
    if record.compatibility_state != "compatible" || record.review_state == "revoked" {
        return Err("This bundle cannot be activated.".into());
    }
    let trust = crate::security::mods::reverify_installed_mod_trust(engine, &record.mod_id)?;
    let connection = engine.open_connection().map_err(|e| e.to_string())?;
    let inspected_payload: String = connection.query_row("SELECT payload_sha256 FROM capability_bundle_records WHERE bundle_id=?1 AND package_version=?2",params![request.bundle_id,request.package_version],|row|row.get(0)).map_err(|e|e.to_string())?;
    if trust.mod_id != record.mod_id
        || trust.version != request.package_version
        || trust.payload_sha256 != inspected_payload
    {
        return Err("This mod changed after review. Review it again before turning it on.".into());
    }
    if trust.integrity_state == "modified" {
        connection.execute("UPDATE capability_bundle_records SET install_state='quarantined',review_state=CASE WHEN review_state='revoked' THEN 'revoked' ELSE 'unreviewed' END,updated_at_ms=?3 WHERE bundle_id=?1 AND package_version=?2",params![request.bundle_id,request.package_version,crate::foundation::clock::unix_time_ms_i64()]).map_err(|e|e.to_string())?;
        return Err(
            "This mod has changed since it was signed. Reinstall it from a trusted source.".into(),
        );
    }
    connection.execute("UPDATE capability_bundle_records SET publisher_identity_verified=?3,review_state=CASE WHEN review_state='revoked' THEN 'revoked' ELSE ?4 END,updated_at_ms=?5 WHERE bundle_id=?1 AND package_version=?2",params![request.bundle_id,request.package_version,trust.publisher_identity_verified,trust.review_state,crate::foundation::clock::unix_time_ms_i64()]).map_err(|e|e.to_string())?;
    record = get_unlocked(engine, &request.bundle_id, &request.package_version)?;
    if record.review_state == "revoked" {
        return Err("This bundle is no longer available for activation.".into());
    }
    if record.review_state != "reviewed" && !request.acknowledge_unreviewed {
        return Err("Confirm that you trust this unreviewed bundle before installing it.".into());
    }
    let now = crate::foundation::clock::unix_time_ms_i64();
    let mut connection = engine.open_connection().map_err(|e| e.to_string())?;
    let tx = connection.transaction().map_err(|e| e.to_string())?;
    let activated = tx.execute("UPDATE capability_bundle_records SET install_state='active',installed_at_ms=COALESCE(installed_at_ms,?3),updated_at_ms=?3 WHERE bundle_id=?1 AND package_version=?2 AND review_state<>'revoked' AND compatibility_state='compatible'",params![request.bundle_id,request.package_version,now]).map_err(|e|e.to_string())?;
    if activated != 1 {
        return Err("This bundle is no longer available for activation.".into());
    }
    let enabled = tx.execute(
        "UPDATE installed_mods SET is_active=1,updated_at_ms=?4 WHERE id=?1 AND version=?2 AND payload_sha256=?3 AND review_state<>'revoked' AND integrity_state<>'modified'",
        params![record.mod_id, request.package_version, inspected_payload, now],
    ).map_err(|e| e.to_string())?;
    if enabled != 1 {
        return Err("This mod changed before activation could finish.".into());
    }
    receipt(
        &tx,
        &request.bundle_id,
        &request.package_version,
        "activated",
        serde_json::json!({
            "unreviewedAcknowledged": request.acknowledge_unreviewed,
            "availability": if record.project_ids.is_empty() { "global" } else { "selected_projects" },
            "projectCount": record.project_ids.len()
        }),
    )?;
    tx.commit().map_err(|e| e.to_string())?;
    get_unlocked(engine, &request.bundle_id, &request.package_version)
}
pub fn disable(
    engine: &PersistenceEngine,
    request: BundleVersionRequest,
) -> Result<CapabilityBundleRecord, String> {
    let _operation_guard = crate::security::mod_trust::lock_mod_package_operation()?;
    let record = get_unlocked(engine, &request.bundle_id, &request.package_version)?;
    let now = crate::foundation::clock::unix_time_ms_i64();
    let mut connection = engine.open_connection().map_err(|e| e.to_string())?;
    let tx = connection.transaction().map_err(|e| e.to_string())?;
    tx.execute("UPDATE capability_bundle_records SET install_state='disabled',updated_at_ms=?3 WHERE bundle_id=?1 AND package_version=?2",params![request.bundle_id,request.package_version,now]).map_err(|e|e.to_string())?;
    tx.execute(
        "UPDATE installed_mods SET is_active=0,updated_at_ms=?2 WHERE id=?1",
        params![record.mod_id, now],
    )
    .map_err(|e| e.to_string())?;
    receipt(
        &tx,
        &request.bundle_id,
        &request.package_version,
        "disabled",
        serde_json::json!({}),
    )?;
    tx.commit().map_err(|e| e.to_string())?;
    get_unlocked(engine, &request.bundle_id, &request.package_version)
}
pub fn authorize(
    engine: &PersistenceEngine,
    request: BundleAuthorityRequest,
) -> Result<(), String> {
    let _operation_guard = crate::security::mod_trust::lock_mod_package_operation()?;
    ProjectId::parse(&request.project_id)?;
    if !CAPABILITY_KINDS.contains(&request.capability.as_str()) {
        return Err("This ability is not supported.".into());
    }
    let connection = engine.open_connection().map_err(|e| e.to_string())?;
    let active_mod: Option<String> = connection.query_row("SELECT mod_id FROM capability_bundle_records WHERE bundle_id=?1 AND install_state='active' ORDER BY installed_at_ms DESC LIMIT 1", params![request.bundle_id], |row| row.get(0)).optional().map_err(|e| e.to_string())?;
    let Some(active_mod) = active_mod else {
        return Err("This bundle is not active.".into());
    };
    crate::security::mods::reverify_installed_mod_trust(engine, &active_mod)?;
    let raw:Option<(String,String,String)>=connection.query_row("SELECT b.capabilities_json,b.project_ids_json,b.install_state FROM capability_bundle_records b JOIN installed_mods m ON m.id=b.mod_id AND m.version=b.package_version AND m.payload_sha256=b.payload_sha256 WHERE b.bundle_id=?1 AND b.install_state='active' AND b.review_state<>'revoked' AND b.compatibility_state='compatible' AND m.is_active=1 AND m.integrity_state<>'modified' ORDER BY b.installed_at_ms DESC LIMIT 1",params![request.bundle_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(|e|e.to_string())?;
    let Some((grants, projects, state)) = raw else {
        return Err("This bundle is not installed.".into());
    };
    let grants: Vec<CapabilityGrant> =
        serde_json::from_str(&grants).map_err(|_| "Bundle abilities are invalid.".to_string())?;
    let projects: Vec<String> = serde_json::from_str(&projects)
        .map_err(|_| "Bundle Project access is invalid.".to_string())?;
    if state == "active"
        && (projects.is_empty() || projects.contains(&request.project_id))
        && grants.iter().any(|g| {
            g.capability == request.capability && g.bounded_scope == request.requested_scope
        })
    {
        return Ok(());
    }
    connection.execute("INSERT INTO capability_runtime_denials (denial_id,bundle_id,project_id,requested_capability,declared_capabilities_json,created_at_ms) VALUES (?1,?2,?3,?4,?5,?6)",params![random_id("denial"),request.bundle_id,request.project_id,request.capability,serde_json::to_string(&grants).map_err(|e|e.to_string())?,crate::foundation::clock::unix_time_ms_i64()]).map_err(|e|e.to_string())?;
    Err("This bundle did not declare permission for that action in this Project.".into())
}
pub fn refresh_catalog(
    engine: &PersistenceEngine,
    request: RegistryCatalogRequest,
) -> Result<Vec<RegistryEntry>, String> {
    let _operation_guard = crate::security::mod_trust::lock_mod_package_operation()?;
    let payload = serde_json::to_vec(&request.catalog).map_err(|e| e.to_string())?;
    crate::security::mod_trust::verify_registry_catalog(
        &request.public_key,
        &payload,
        &request.signature,
    )?;
    let catalog_signature = request.signature.clone();
    if request.catalog.entries.len() > 500 {
        return Err("The registry catalog is too large.".into());
    }
    let now = crate::foundation::clock::unix_time_ms_i64();
    let mut connection = engine.open_connection().map_err(|e| e.to_string())?;
    let tx = connection.transaction().map_err(|e| e.to_string())?;
    for item in request.catalog.entries {
        if !matches!(
            item.review_state.as_str(),
            "reviewed" | "unreviewed" | "revoked"
        ) || !valid_version(&item.package_version)
            || item.payload_sha256.len() != 64
            || !item
                .payload_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err("The registry contains invalid package information.".into());
        }
        tx.execute("INSERT INTO capability_registry_entries (bundle_id,package_version,catalog_revision,name,summary,category,publisher_name,review_state,compatibility_state,changelog,metadata_sha256,metadata_signature,cached_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13) ON CONFLICT(bundle_id,package_version) DO UPDATE SET catalog_revision=excluded.catalog_revision,name=excluded.name,summary=excluded.summary,category=excluded.category,publisher_name=excluded.publisher_name,review_state=excluded.review_state,compatibility_state=excluded.compatibility_state,changelog=excluded.changelog,metadata_sha256=excluded.metadata_sha256,metadata_signature=excluded.metadata_signature,cached_at_ms=excluded.cached_at_ms",params![item.bundle_id,item.package_version,request.catalog.revision,item.name,item.summary,item.category,item.publisher_name,item.review_state,item.compatibility_state,item.changelog,item.payload_sha256,catalog_signature,now]).map_err(|e|e.to_string())?;
        if item.review_state == "revoked" {
            tx.execute("UPDATE capability_bundle_records SET review_state='revoked',install_state='blocked',updated_at_ms=?3 WHERE bundle_id=?1 AND package_version=?2",params![item.bundle_id,item.package_version,now]).map_err(|e|e.to_string())?;
            tx.execute("UPDATE installed_mods SET review_state='revoked',is_active=0,updated_at_ms=?3 WHERE id IN (SELECT mod_id FROM capability_bundle_records WHERE bundle_id=?1 AND package_version=?2)",params![item.bundle_id,item.package_version,now]).map_err(|e|e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    registry(engine)
}
pub fn registry(engine: &PersistenceEngine) -> Result<Vec<RegistryEntry>, String> {
    let connection = engine.open_connection().map_err(|e| e.to_string())?;
    let mut statement=connection.prepare("SELECT r.bundle_id,r.package_version,r.name,r.summary,r.category,r.publisher_name,r.review_state,r.compatibility_state,r.changelog,EXISTS(SELECT 1 FROM capability_bundle_records b WHERE b.bundle_id=r.bundle_id AND b.package_version=r.package_version AND b.install_state IN ('active','disabled')) FROM capability_registry_entries r ORDER BY r.review_state='reviewed' DESC,r.name").map_err(|e|e.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(RegistryEntry {
                bundle_id: row.get(0)?,
                package_version: row.get(1)?,
                name: row.get(2)?,
                summary: row.get(3)?,
                category: row.get(4)?,
                publisher_name: row.get(5)?,
                review_state: row.get(6)?,
                compatibility_state: row.get(7)?,
                changelog: row.get(8)?,
                installed: row.get::<_, i64>(9)? != 0,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}
