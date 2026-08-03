use super::{crypto, SignedRemoteCommand};
use crate::{
    db::PersistenceEngine,
    p0_contracts::{ProjectId, TaskRunId},
};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::Value;

const COMMAND_MAX_TTL_MS: i64 = 5 * 60 * 1000;

pub(crate) const REMOTE_COMMAND_SCHEMA: [(&str, bool); 16] = [
    ("command_id", true),
    ("remote_device_id", true),
    ("project_id", true),
    ("task_run_id", false),
    ("command_kind", true),
    ("nonce", true),
    ("expires_at_ms", true),
    ("expected_task_sequence", false),
    ("payload_sha256", true),
    ("signer_public_key", true),
    ("signature", true),
    ("status", true),
    ("outcome_code", false),
    ("result_json", false),
    ("received_at_ms", true),
    ("completed_at_ms", false),
];

fn remote_command_columns() -> String {
    REMOTE_COMMAND_SCHEMA
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(",")
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StoredRemoteCommand {
    pub command_id: String,
    pub remote_device_id: String,
    pub project_id: String,
    pub task_run_id: Option<String>,
    pub command_kind: String,
    pub nonce: String,
    pub expires_at_ms: i64,
    pub expected_task_sequence: Option<u64>,
    pub payload_sha256: String,
    pub signer_public_key: String,
    pub signature: String,
    pub status: String,
    pub outcome_code: Option<String>,
    pub result_json: Option<Value>,
    pub received_at_ms: i64,
    pub completed_at_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommandStoreError {
    pub code: &'static str,
    pub message: String,
}

impl std::fmt::Display for CommandStoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for CommandStoreError {}

#[derive(Clone, Debug)]
pub(crate) enum CommandAcceptance {
    Accepted(StoredRemoteCommand),
    SequenceConflict {
        command: StoredRemoteCommand,
        message: String,
    },
}

fn error(code: &'static str, message: impl Into<String>) -> CommandStoreError {
    CommandStoreError {
        code,
        message: message.into(),
    }
}

fn supported_scope(kind: &str) -> Option<&'static str> {
    match kind {
        "view_task" => Some("view_task"),
        "stop_task" => Some("stop_task"),
        "request_artifact" => Some("request_artifact"),
        "create_task" => Some("create_task"),
        "steer_task" => Some("steer_task"),
        "answer_clarification" => Some("answer_clarification"),
        "approve_bounded_action" => Some("approve_bounded_action"),
        _ => None,
    }
}

pub(crate) fn canonical(command: &SignedRemoteCommand) -> String {
    serde_json::json!({
        "commandId": command.command_id,
        "remoteDeviceId": command.remote_device_id,
        "projectId": command.project_id,
        "taskRunId": command.task_run_id,
        "commandKind": command.command_kind,
        "nonce": command.nonce,
        "expiresAtMs": command.expires_at_ms,
        "expectedTaskSequence": command.expected_task_sequence,
        "payloadSha256": command.payload_sha256,
        "signerPublicKey": command.signer_public_key,
    })
    .to_string()
}

fn decode(row: &Row<'_>) -> rusqlite::Result<StoredRemoteCommand> {
    let expected = row.get::<_, Option<i64>>(7)?.map(|value| value as u64);
    let result = row
        .get::<_, Option<String>>(13)?
        .map(|encoded| {
            serde_json::from_str(&encoded).map_err(|cause| {
                rusqlite::Error::FromSqlConversionFailure(
                    13,
                    rusqlite::types::Type::Text,
                    Box::new(cause),
                )
            })
        })
        .transpose()?;
    Ok(StoredRemoteCommand {
        command_id: row.get(0)?,
        remote_device_id: row.get(1)?,
        project_id: row.get(2)?,
        task_run_id: row.get(3)?,
        command_kind: row.get(4)?,
        nonce: row.get(5)?,
        expires_at_ms: row.get(6)?,
        expected_task_sequence: expected,
        payload_sha256: row.get(8)?,
        signer_public_key: row.get(9)?,
        signature: row.get(10)?,
        status: row.get(11)?,
        outcome_code: row.get(12)?,
        result_json: result,
        received_at_ms: row.get(14)?,
        completed_at_ms: row.get(15)?,
    })
}

pub(crate) fn load(
    connection: &Connection,
    command_id: &str,
) -> Result<Option<StoredRemoteCommand>, CommandStoreError> {
    connection
        .query_row(
            &format!(
                "SELECT {} FROM remote_commands WHERE command_id=?1",
                remote_command_columns()
            ),
            params![command_id],
            decode,
        )
        .optional()
        .map_err(|cause| error("remote_command_store_read_failed", cause.to_string()))
}

fn current_task_sequence(
    connection: &Connection,
    task_run_id: &str,
) -> Result<u64, CommandStoreError> {
    let value = connection
        .query_row(
            "SELECT COALESCE(MAX(sequence),-1)+1 FROM task_events WHERE task_run_id=?1",
            params![task_run_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|cause| error("remote_command_sequence_read_failed", cause.to_string()))?;
    u64::try_from(value).map_err(|_| {
        error(
            "remote_command_sequence_invalid",
            "The Task sequence is invalid.",
        )
    })
}

fn validate_hex(value: &str, bytes: usize) -> bool {
    value.len() == bytes * 2
        && hex::decode(value)
            .map(|decoded| decoded.len() == bytes)
            .unwrap_or(false)
}

fn duplicate_error(
    connection: &Connection,
    command: &SignedRemoteCommand,
) -> Result<Option<CommandStoreError>, CommandStoreError> {
    let command_exists = connection
        .query_row(
            "SELECT 1 FROM remote_commands WHERE command_id=?1 LIMIT 1",
            params![command.command_id],
            |_| Ok(()),
        )
        .optional()
        .map_err(|cause| error("remote_command_store_read_failed", cause.to_string()))?
        .is_some();
    if command_exists {
        return Ok(Some(error(
            "remote_command_duplicate_id",
            "This remote request was already received.",
        )));
    }
    let nonce_exists = connection
        .query_row(
            "SELECT 1 FROM remote_commands WHERE remote_device_id=?1 AND nonce=?2 LIMIT 1",
            params![command.remote_device_id, command.nonce],
            |_| Ok(()),
        )
        .optional()
        .map_err(|cause| error("remote_command_store_read_failed", cause.to_string()))?
        .is_some();
    Ok(nonce_exists.then(|| {
        error(
            "remote_command_replayed_nonce",
            "This remote request cannot be used again.",
        )
    }))
}

pub(crate) fn accept(
    engine: &PersistenceEngine,
    command: &SignedRemoteCommand,
) -> Result<CommandAcceptance, CommandStoreError> {
    let now = crate::foundation::clock::unix_time_ms_i64();
    if command.command_id.trim().is_empty() || command.command_id.len() > 160 {
        return Err(error(
            "remote_command_identity_invalid",
            "Remote request identity is invalid.",
        ));
    }
    if !validate_hex(&command.nonce, 32) {
        return Err(error(
            "remote_command_nonce_invalid",
            "Remote request identity is invalid.",
        ));
    }
    ProjectId::parse(&command.project_id)
        .map_err(|message| error("remote_command_project_invalid", message))?;
    if let Some(task_run_id) = command.task_run_id.as_deref() {
        TaskRunId::parse(task_run_id)
            .map_err(|message| error("remote_command_task_invalid", message))?;
    }
    let required_scope = supported_scope(&command.command_kind).ok_or_else(|| {
        error(
            "remote_command_kind_unsupported",
            "This remote action is not supported.",
        )
    })?;
    let encoded_payload = serde_json::to_vec(&command.payload)
        .map_err(|cause| error("remote_command_payload_invalid", cause.to_string()))?;
    let payload_sha256 = crate::foundation::digest::sha256_hex(&encoded_payload);
    if !validate_hex(&command.payload_sha256, 32) || command.payload_sha256 != payload_sha256 {
        return Err(error(
            "remote_command_payload_digest_mismatch",
            "The remote request changed after it was signed.",
        ));
    }

    let connection = engine
        .open_connection()
        .map_err(|cause| error("remote_command_store_unavailable", cause.to_string()))?;
    let device = connection
        .query_row(
            "SELECT public_key,allowed_project_ids_json,scopes_json,revoked_at_ms,expires_at_ms FROM remote_devices WHERE remote_device_id=?1",
            params![command.remote_device_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .optional()
        .map_err(|cause| error("remote_command_store_read_failed", cause.to_string()))?
        .ok_or_else(|| {
            error(
                "remote_command_device_unknown",
                "Paired device was not found.",
            )
        })?;
    if device.3.is_some() {
        return Err(error(
            "remote_command_device_revoked",
            "This device no longer has access.",
        ));
    }
    if device.4 < now {
        return Err(error(
            "remote_command_device_expired",
            "This device needs to be paired again.",
        ));
    }
    if command.signer_public_key != device.0 {
        return Err(error(
            "remote_command_signer_key_mismatch",
            "The request was not signed by this paired device.",
        ));
    }
    crypto::verify(&device.0, &canonical(command), &command.signature).map_err(|_| {
        error(
            "remote_command_signature_mismatch",
            "The remote request signature could not be verified.",
        )
    })?;
    if command.expires_at_ms < now {
        return Err(error(
            "remote_command_expired",
            "This remote request expired.",
        ));
    }
    if command.expires_at_ms > now.saturating_add(COMMAND_MAX_TTL_MS) {
        return Err(error(
            "remote_command_expiry_unbounded",
            "This remote request lasts too long.",
        ));
    }

    let allowed_projects: Vec<String> = serde_json::from_str(&device.1).map_err(|_| {
        error(
            "remote_command_device_projects_invalid",
            "Device Project access is invalid.",
        )
    })?;
    let allowed_scopes: Vec<String> = serde_json::from_str(&device.2).map_err(|_| {
        error(
            "remote_command_device_scopes_invalid",
            "Device permissions are invalid.",
        )
    })?;
    let project_exists = connection
        .query_row(
            "SELECT 1 FROM projects WHERE project_id=?1 LIMIT 1",
            params![command.project_id],
            |_| Ok(()),
        )
        .optional()
        .map_err(|cause| error("remote_command_store_read_failed", cause.to_string()))?
        .is_some();
    if !project_exists
        || !allowed_projects
            .iter()
            .any(|value| value == &command.project_id)
        || !allowed_scopes.iter().any(|value| value == required_scope)
    {
        return Err(error(
            "remote_command_project_scope_mismatch",
            "This device is not allowed to do that in this Project.",
        ));
    }

    if matches!(command.command_kind.as_str(), "view_task" | "stop_task") {
        let task_run_id = command.task_run_id.as_deref().ok_or_else(|| {
            error(
                "remote_command_task_required",
                "Choose a Task for this request.",
            )
        })?;
        let task_project = connection
            .query_row(
                "SELECT project_id FROM task_runs WHERE task_run_id=?1",
                params![task_run_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(|cause| error("remote_command_store_read_failed", cause.to_string()))?
            .flatten();
        if task_project.as_deref() != Some(command.project_id.as_str()) {
            return Err(error(
                "remote_command_task_scope_mismatch",
                "This Task is outside the device's Project access.",
            ));
        }
    }

    if command.command_kind == "request_artifact" {
        let artifact_id = command
            .payload
            .get("artifactId")
            .and_then(Value::as_str)
            .ok_or_else(|| error("remote_command_artifact_required", "Choose a file to send."))?;
        let artifact_project = connection
            .query_row(
                "SELECT project_id FROM artifact_records WHERE artifact_id=?1",
                params![artifact_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|cause| error("remote_command_store_read_failed", cause.to_string()))?;
        if artifact_project.as_deref() != Some(command.project_id.as_str()) {
            return Err(error(
                "remote_command_artifact_scope_mismatch",
                "This file is outside the device's Project access.",
            ));
        }
    }

    if let Some(duplicate) = duplicate_error(&connection, command)? {
        return Err(duplicate);
    }

    let sequence_conflict = if command.command_kind == "stop_task" {
        let expected = command.expected_task_sequence.ok_or_else(|| {
            error(
                "remote_command_sequence_required",
                "Refresh this Task before stopping it.",
            )
        })?;
        let current = current_task_sequence(
            &connection,
            command.task_run_id.as_deref().expect("validated task id"),
        )?;
        expected != current
    } else {
        false
    };
    let expected_sequence = command
        .expected_task_sequence
        .map(i64::try_from)
        .transpose()
        .map_err(|_| {
            error(
                "remote_command_sequence_invalid",
                "The Task sequence is invalid.",
            )
        })?;
    let insert = connection.execute(
        &format!(
            "INSERT INTO remote_commands ({}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
            remote_command_columns()
        ),
        params![
            command.command_id,
            command.remote_device_id,
            command.project_id,
            command.task_run_id,
            command.command_kind,
            command.nonce,
            command.expires_at_ms,
            expected_sequence,
            command.payload_sha256,
            command.signer_public_key,
            command.signature,
            "accepted",
            Option::<String>::None,
            Option::<String>::None,
            now,
            Option::<i64>::None,
        ],
    );
    if let Err(cause) = insert {
        if let Some(duplicate) = duplicate_error(&connection, command)? {
            return Err(duplicate);
        }
        return Err(error(
            "remote_command_store_write_failed",
            cause.to_string(),
        ));
    }
    let stored = load(&connection, &command.command_id)?.ok_or_else(|| {
        error(
            "remote_command_store_write_failed",
            "The accepted request could not be read back.",
        )
    })?;
    if sequence_conflict {
        Ok(CommandAcceptance::SequenceConflict {
            command: stored,
            message: "This Task changed on your Mac. OOMU kept the Mac's version.".to_string(),
        })
    } else {
        Ok(CommandAcceptance::Accepted(stored))
    }
}

#[cfg(test)]
pub(crate) fn schema_contract(
    connection: &Connection,
) -> Result<Vec<(String, bool)>, CommandStoreError> {
    let mut statement = connection
        .prepare("PRAGMA table_info(remote_commands)")
        .map_err(|cause| error("remote_command_schema_read_failed", cause.to_string()))?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, i64>(3)? != 0))
        })
        .map_err(|cause| error("remote_command_schema_read_failed", cause.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|cause| error("remote_command_schema_read_failed", cause.to_string()))?;
    Ok(rows)
}
