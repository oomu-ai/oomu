use crate::{db::PersistenceEngine, foundation::clock::unix_time_ms_i64};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrivateEgressConfirmationRequest {
    pub session_id: String,
    pub turn_id: String,
    pub generation_token: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvePrivateEgressConfirmationRequest {
    pub challenge_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub generation_token: String,
    pub approved: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivateEgressConfirmationView {
    pub challenge_id: String,
    pub destination_provider_id: String,
    pub destination_model_id: String,
    pub source_names: Vec<String>,
    pub allowed_representation: String,
    pub expires_at_ms: i64,
    pub decision: String,
}

fn clean(value: &str) -> Result<&str, String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 256 {
        return Err("This send decision no longer matches the original request.".to_string());
    }
    Ok(value)
}

fn select(
    engine: &PersistenceEngine,
    request: &PrivateEgressConfirmationRequest,
) -> Result<PrivateEgressConfirmationView, String> {
    let session_id = clean(&request.session_id)?;
    let turn_id = clean(&request.turn_id)?;
    let generation_token = clean(&request.generation_token)?;
    let row = engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .query_row(
            "SELECT challenge_id,destination_provider_id,destination_model_id,
                    source_names_json,allowed_representation,expires_at_ms,decision
             FROM private_egress_confirmation_challenges
             WHERE session_id=?1 AND turn_id=?2 AND generation_token=?3",
            params![session_id, turn_id, generation_token],
            |row| {
                let source_names_json: String = row.get(3)?;
                let source_names = serde_json::from_str(&source_names_json).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        3,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?;
                Ok(PrivateEgressConfirmationView {
                    challenge_id: row.get(0)?,
                    destination_provider_id: row.get(1)?,
                    destination_model_id: row.get(2)?,
                    source_names,
                    allowed_representation: row.get(4)?,
                    expires_at_ms: row.get(5)?,
                    decision: row.get(6)?,
                })
            },
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "This send decision is no longer available.".to_string())?;
    if row.expires_at_ms < unix_time_ms_i64() {
        return Err("This send decision expired. Review the request again.".to_string());
    }
    Ok(row)
}

#[tauri::command]
pub async fn get_private_egress_confirmation(
    request: PrivateEgressConfirmationRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<PrivateEgressConfirmationView, String> {
    select(persistence.inner(), &request)
}

#[tauri::command]
pub async fn resolve_private_egress_confirmation(
    request: ResolvePrivateEgressConfirmationRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<PrivateEgressConfirmationView, String> {
    let lookup = PrivateEgressConfirmationRequest {
        session_id: request.session_id.clone(),
        turn_id: request.turn_id.clone(),
        generation_token: request.generation_token.clone(),
    };
    let current = select(persistence.inner(), &lookup)?;
    if current.challenge_id != clean(&request.challenge_id)? {
        return Err("This send decision no longer matches the original request.".to_string());
    }
    let decision = if request.approved {
        "approved"
    } else {
        "denied"
    };
    let changed = persistence
        .open_connection()
        .map_err(|error| error.to_string())?
        .execute(
            "UPDATE private_egress_confirmation_challenges
             SET decision=?2,decided_at_ms=?3
             WHERE challenge_id=?1 AND decision='pending' AND expires_at_ms>=?3
               AND session_id=?4 AND turn_id=?5 AND generation_token=?6",
            params![
                current.challenge_id,
                decision,
                unix_time_ms_i64(),
                clean(&request.session_id)?,
                clean(&request.turn_id)?,
                clean(&request.generation_token)?,
            ],
        )
        .map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("This send decision was already used or expired.".to_string());
    }
    select(persistence.inner(), &lookup)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_rejects_empty_or_unbounded_identity_fields() {
        assert!(clean("").is_err());
        assert!(clean(&"x".repeat(257)).is_err());
        assert_eq!(clean(" turn-1 ").unwrap(), "turn-1");
    }
}
