use super::{receipts::insert_receipt, state};
use crate::{db::PersistenceEngine, foundation::digest::sha256_hex};
use rusqlite::params;
use serde_json::Value;
use std::{collections::BTreeMap, fs};

#[derive(Clone, Debug, PartialEq, Eq)]
struct RequiredFileEffect {
    idempotency_key: String,
    result_digest: String,
}

pub(super) fn record_verified_schedule_completion(
    engine: &PersistenceEngine,
    schedule_id: &str,
    task_run_id: &str,
) -> Result<bool, String> {
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let required = match required_file_effects(&connection, task_run_id) {
        Ok(required) => required,
        Err(code) if code.starts_with("scheduled_file_effect_") => return Ok(false),
        Err(error) => return Err(error),
    };
    if required.is_empty() {
        return Ok(false);
    }
    let mut statement = connection
        .prepare(
            "SELECT event_json FROM task_events WHERE task_run_id=?1 AND json_extract(event_json,'$.evidenceClass')='verified_postcondition' AND json_extract(event_json,'$.eventType')='workflow.effect.verified' AND json_extract(event_json,'$.payload.effectKind')='create_file' ORDER BY sequence",
        )
        .map_err(|error| error.to_string())?;
    let encoded = statement
        .query_map(params![task_run_id], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    drop(statement);
    let events = encoded
        .into_iter()
        .filter_map(|item| serde_json::from_str::<Value>(&item).ok())
        .collect::<Vec<_>>();
    let Some(verified_digests) = verified_effect_digests(&required, &events) else {
        return Ok(false);
    };
    let row = state::load_row(&connection)?;
    insert_receipt(
        &connection,
        &row,
        "scheduled_postcondition_verified",
        "verified",
        None,
        Some(&sha256_hex(schedule_id.as_bytes())),
        Some(&sha256_hex(verified_digests.join("\n").as_bytes())),
    )?;
    Ok(true)
}

fn required_file_effects(
    connection: &rusqlite::Connection,
    task_run_id: &str,
) -> Result<Vec<RequiredFileEffect>, String> {
    let mut statement = connection
        .prepare("SELECT idempotency_key,result_digest,state FROM task_effects WHERE task_run_id=?1 AND effect_kind='create_file' ORDER BY idempotency_key")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![task_run_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    rows.into_iter()
        .map(|(idempotency_key, result_digest, state)| {
            if state != "verified" {
                return Err("scheduled_file_effect_not_verified".to_string());
            }
            Ok(RequiredFileEffect {
                idempotency_key,
                result_digest: result_digest
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| "scheduled_file_effect_digest_missing".to_string())?,
            })
        })
        .collect()
}

fn verified_effect_digests(
    required: &[RequiredFileEffect],
    events: &[Value],
) -> Option<Vec<String>> {
    let expected = required
        .iter()
        .map(|effect| {
            (
                effect.idempotency_key.as_str(),
                effect.result_digest.as_str(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut observed = BTreeMap::new();
    for event in events {
        let payload = event.get("payload")?;
        let key = payload.get("idempotencyKey").and_then(Value::as_str)?;
        let receipt_digest = payload.get("resultDigest").and_then(Value::as_str)?;
        if expected.get(key).copied() != Some(receipt_digest) || observed.contains_key(key) {
            return None;
        }
        observed.insert(key, verified_file_digest(event)?);
    }
    if observed.len() != expected.len() || expected.keys().any(|key| !observed.contains_key(key)) {
        return None;
    }
    Some(
        observed
            .into_iter()
            .map(|(key, digest)| format!("{}:{digest}", sha256_hex(key.as_bytes())))
            .collect(),
    )
}

pub(super) fn scheduled_file_postcondition_required(
    engine: &PersistenceEngine,
    task_run_id: &str,
) -> Result<bool, String> {
    engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM task_effects WHERE task_run_id=?1 AND effect_kind='create_file')",
            params![task_run_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}

fn verified_file_digest(event: &Value) -> Option<String> {
    let payload = event.get("payload")?;
    let evidence = payload.get("result")?;
    let path = evidence
        .get("path")
        .or_else(|| evidence.pointer("/structuredContent/path"))
        .and_then(Value::as_str)?;
    let expected_sha = evidence
        .get("sha256")
        .or_else(|| evidence.pointer("/structuredContent/sha256"))
        .and_then(Value::as_str)?;
    let expected_len = evidence
        .get("byteLength")
        .or_else(|| evidence.pointer("/structuredContent/byteLength"))
        .and_then(Value::as_u64)?;
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != expected_len {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    (sha256_hex(&bytes) == expected_sha).then(|| expected_sha.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduled_file_postcondition_is_reread_from_disk() {
        let root = std::env::temp_dir().join(format!(
            "oomu-background-postcondition-{}",
            crate::p0_contracts::TaskId::new()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("result.md");
        std::fs::write(&path, b"verified result").unwrap();
        let digest = sha256_hex(b"verified result");
        let event = serde_json::json!({
            "eventType": "workflow.effect.verified",
            "payload": { "effectKind":"create_file", "result": {
                "path": path, "sha256": digest, "byteLength": 15
            }}
        });
        assert_eq!(verified_file_digest(&event), Some(digest));
        std::fs::write(&path, b"changed result").unwrap();
        assert_eq!(verified_file_digest(&event), None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn every_reserved_file_requires_its_exact_verified_receipt() {
        let root = std::env::temp_dir().join(format!(
            "oomu-background-effect-set-{}",
            crate::p0_contracts::TaskId::new()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut required = Vec::new();
        let mut events = Vec::new();
        for index in 1..=2 {
            let path = root.join(format!("result-{index}.md"));
            let bytes = format!("verified result {index}");
            std::fs::write(&path, bytes.as_bytes()).unwrap();
            let result = serde_json::json!({
                "path":path,"sha256":sha256_hex(bytes.as_bytes()),"byteLength":bytes.len()
            });
            let receipt_digest = sha256_hex(&serde_json::to_vec(&result).unwrap());
            let key = format!("effect-{index}");
            required.push(RequiredFileEffect {
                idempotency_key: key.clone(),
                result_digest: receipt_digest.clone(),
            });
            events.push(serde_json::json!({"payload":{
                "idempotencyKey":key,"effectKind":"create_file",
                "resultDigest":receipt_digest,"result":result
            }}));
        }
        assert_eq!(
            verified_effect_digests(&required, &events).unwrap().len(),
            2
        );
        events.pop();
        assert!(verified_effect_digests(&required, &events).is_none());
        let _ = std::fs::remove_dir_all(root);
    }
}
