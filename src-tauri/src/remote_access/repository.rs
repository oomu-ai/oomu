use super::*;
use crate::db::PersistenceEngine;
#[cfg(test)]
use crate::p0_contracts::ProjectId;
#[cfg(test)]
use qrcode::{render::svg, QrCode};
use rusqlite::{params, OptionalExtension, Row};

#[cfg(test)]
const PAIRING_TTL_MS: i64 = 2 * 60 * 1000;

#[cfg(test)]
fn projects_exist(engine: &PersistenceEngine, ids: &[String]) -> Result<(), String> {
    if ids.is_empty() || ids.len() > 128 {
        return Err("Choose at least one Project.".into());
    }
    let connection = engine.open_connection().map_err(|e| e.to_string())?;
    for id in ids {
        ProjectId::parse(id)?;
        let exists = connection
            .query_row(
                "SELECT 1 FROM projects WHERE project_id=?1 AND archived_at_ms IS NULL",
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
    Ok(())
}
#[cfg(test)]
fn valid_scopes(scopes: &[String]) -> Result<(), String> {
    if scopes.is_empty() || scopes.len() > REMOTE_SCOPES.len() {
        return Err("Choose what this device can do.".into());
    }
    if scopes
        .iter()
        .any(|scope| !REMOTE_SCOPES.contains(&scope.as_str()))
    {
        return Err("A device permission is not supported.".into());
    }
    Ok(())
}

#[cfg(test)]
pub fn create_challenge(
    engine: &PersistenceEngine,
    request: CreatePairingChallengeRequest,
) -> Result<PairingChallenge, String> {
    engine.require_durable_store("pair a remote device")?;
    projects_exist(engine, &request.allowed_project_ids)?;
    valid_scopes(&request.scopes)?;
    let challenge_id = crypto::random_hex(16);
    let secret = crypto::random_hex(32);
    let now = crate::foundation::clock::unix_time_ms_i64();
    let expires = now + PAIRING_TTL_MS;
    let payload = format!("oomu://pair?challenge={challenge_id}&secret={secret}");
    let qr = QrCode::new(payload.as_bytes())
        .map_err(|_| "The pairing code could not be created.".to_string())?
        .render::<svg::Color>()
        .min_dimensions(240, 240)
        .dark_color(svg::Color("#202124"))
        .light_color(svg::Color("#ffffff"))
        .build();
    engine.open_connection().map_err(|e|e.to_string())?.execute("INSERT INTO remote_pairing_challenges (challenge_id,secret_hash,qr_payload,requested_scopes_json,allowed_project_ids_json,expires_at_ms,created_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7)",params![challenge_id,crate::foundation::digest::sha256_hex(secret.as_bytes()),payload,serde_json::to_string(&request.scopes).map_err(|e|e.to_string())?,serde_json::to_string(&request.allowed_project_ids).map_err(|e|e.to_string())?,expires,now]).map_err(|e|e.to_string())?;
    Ok(PairingChallenge {
        challenge_id,
        qr_svg: qr,
        expires_at_ms: expires,
        status: "waiting_for_scan".into(),
    })
}

#[cfg(test)]
pub fn submit_response(
    engine: &PersistenceEngine,
    request: SubmitPairingResponseRequest,
) -> Result<(), String> {
    let now = crate::foundation::clock::unix_time_ms_i64();
    if request.device_label.trim().is_empty() || request.device_label.chars().count() > 80 {
        return Err("Device name is required.".into());
    }
    if request.public_key.len() != 64
        || hex::decode(&request.public_key)
            .map(|v| v.len() != 32)
            .unwrap_or(true)
    {
        return Err("Device identity is invalid.".into());
    }
    let changed=engine.open_connection().map_err(|e|e.to_string())?.execute("UPDATE remote_pairing_challenges SET pending_device_label=?3,pending_public_key=?4,response_received_at_ms=?5 WHERE challenge_id=?1 AND secret_hash=?2 AND consumed_at_ms IS NULL AND expires_at_ms>=?5",params![request.challenge_id,crate::foundation::digest::sha256_hex(request.secret.as_bytes()),request.device_label.trim(),request.public_key,now]).map_err(|e|e.to_string())?;
    if changed != 1 {
        return Err("This pairing code expired or was already used.".into());
    }
    Ok(())
}

#[cfg(test)]
pub fn confirm(
    engine: &PersistenceEngine,
    request: ConfirmPairingRequest,
) -> Result<Option<RemoteDeviceRecord>, String> {
    let now = crate::foundation::clock::unix_time_ms_i64();
    let connection = engine.open_connection().map_err(|e| e.to_string())?;
    let pending:Option<(String,String,String,String,i64)>=connection.query_row("SELECT pending_device_label,pending_public_key,allowed_project_ids_json,requested_scopes_json,expires_at_ms FROM remote_pairing_challenges WHERE challenge_id=?1 AND consumed_at_ms IS NULL AND response_received_at_ms IS NOT NULL",params![request.challenge_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?))).optional().map_err(|e|e.to_string())?;
    let Some((label, key, projects, scopes, expires)) = pending else {
        return Err("No device is waiting for confirmation.".into());
    };
    if expires < now {
        return Err("This pairing code expired.".into());
    }
    connection
        .execute(
            "UPDATE remote_pairing_challenges SET consumed_at_ms=?2 WHERE challenge_id=?1",
            params![request.challenge_id, now],
        )
        .map_err(|e| e.to_string())?;
    if !request.allow {
        return Ok(None);
    }
    let id = crypto::uuid_id("device");
    let device_expires = now + 365 * 24 * 60 * 60 * 1000;
    connection.execute("INSERT INTO remote_devices (remote_device_id,label,public_key,allowed_project_ids_json,scopes_json,paired_at_ms,expires_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7)",params![id,label,key,projects,scopes,now,device_expires]).map_err(|e|e.to_string())?;
    get(engine, &id).map(Some)
}

fn from_row(row: &Row<'_>) -> rusqlite::Result<RemoteDeviceRecord> {
    let projects: String = row.get(2)?;
    let scopes: String = row.get(3)?;
    Ok(RemoteDeviceRecord {
        remote_device_id: row.get(0)?,
        label: row.get(1)?,
        allowed_project_ids: serde_json::from_str(&projects).unwrap_or_default(),
        scopes: serde_json::from_str(&scopes).unwrap_or_default(),
        paired_at_ms: row.get(4)?,
        expires_at_ms: row.get(5)?,
        last_used_at_ms: row.get(6)?,
        revoked_at_ms: row.get(7)?,
    })
}
const SELECT:&str="SELECT remote_device_id,label,allowed_project_ids_json,scopes_json,paired_at_ms,expires_at_ms,last_used_at_ms,revoked_at_ms FROM remote_devices";
pub fn list(engine: &PersistenceEngine) -> Result<Vec<RemoteDeviceRecord>, String> {
    let connection = engine.open_connection().map_err(|e| e.to_string())?;
    let mut statement = connection
        .prepare(&format!(
            "{SELECT} ORDER BY revoked_at_ms IS NOT NULL,last_used_at_ms DESC,paired_at_ms DESC"
        ))
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([], from_row)
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}
pub fn get(engine: &PersistenceEngine, id: &str) -> Result<RemoteDeviceRecord, String> {
    crate::p1_contracts::RemoteDeviceId::parse(id)?;
    engine
        .open_connection()
        .map_err(|e| e.to_string())?
        .query_row(
            &format!("{SELECT} WHERE remote_device_id=?1"),
            params![id],
            from_row,
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Paired device was not found.".into())
}
pub fn rename(
    engine: &PersistenceEngine,
    request: RenameRemoteDeviceRequest,
) -> Result<RemoteDeviceRecord, String> {
    let label = request.label.trim();
    if label.is_empty() || label.chars().count() > 80 {
        return Err("Device name is required.".into());
    }
    let changed=engine.open_connection().map_err(|e|e.to_string())?.execute("UPDATE remote_devices SET label=?2 WHERE remote_device_id=?1 AND revoked_at_ms IS NULL",params![request.remote_device_id,label]).map_err(|e|e.to_string())?;
    if changed != 1 {
        return Err("Paired device was not found.".into());
    }
    get(engine, &request.remote_device_id)
}
pub fn revoke(engine: &PersistenceEngine, id: &str) -> Result<RemoteDeviceRecord, String> {
    get(engine, id)?;
    let now = crate::foundation::clock::unix_time_ms_i64();
    let connection = engine.open_connection().map_err(|e| e.to_string())?;
    connection.execute("UPDATE remote_devices SET revoked_at_ms=COALESCE(revoked_at_ms,?2) WHERE remote_device_id=?1",params![id,now]).map_err(|e|e.to_string())?;
    connection.execute("UPDATE remote_artifact_grants SET revoked_at_ms=?2 WHERE remote_device_id=?1 AND revoked_at_ms IS NULL",params![id,now]).map_err(|e|e.to_string())?;
    get(engine, id)
}

pub fn execute(
    engine: &PersistenceEngine,
    identity: &crate::sovereign_identity::SovereignIdentity,
    command: SignedRemoteCommand,
) -> Result<RemoteCommandResult, String> {
    match super::command_store::accept(engine, &command).map_err(|error| error.to_string())? {
        super::command_store::CommandAcceptance::Accepted(stored) => {
            super::execution_commit::execute_accepted(engine, identity, &command, &stored)
        }
        super::command_store::CommandAcceptance::SequenceConflict { command, message } => {
            super::execution_commit::commit_sequence_conflict(engine, identity, &command, &message)
        }
    }
}
