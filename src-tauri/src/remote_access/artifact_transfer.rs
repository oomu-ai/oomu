use super::{
    crypto, EncryptedRemoteArtifact, RemoteArtifactGrant, RetrieveRemoteArtifactRequest,
    SignedRemoteCommand,
};
use crate::{db::PersistenceEngine, sovereign_identity::SovereignIdentity};
use base64::{engine::general_purpose::STANDARD, Engine};
use chacha20poly1305::{
    aead::{Aead, AeadCore, OsRng as AeadOsRng, Payload},
    ChaCha20Poly1305, Key, KeyInit,
};
use rusqlite::{params, OptionalExtension, Transaction};
use sha2::{Digest, Sha256};
use std::path::Path;

const GRANT_TTL_MS: i64 = 10 * 60 * 1000;
const FULL_CONTENT: &str = "full_content";

#[derive(Clone, Debug)]
pub(crate) struct PreparedReceipt {
    pub receipt_id: String,
    pub remote_device_id: String,
    pub command_id: String,
    pub receipt_kind: String,
    pub payload_sha256: String,
    pub signer_public_key: String,
    pub signature: String,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedArtifactGrant {
    pub grant_id: String,
    pub token: String,
    pub token_hash: String,
    pub remote_device_id: String,
    pub project_id: String,
    pub artifact_id: String,
    pub format: String,
    pub source_path: String,
    pub transfer_path: String,
    pub source_sha256: String,
    pub transfer_sha256: String,
    pub content_state: String,
    pub protected: bool,
    pub expires_at_ms: i64,
    pub created_at_ms: i64,
    pub approval_receipt: PreparedReceipt,
}

impl PreparedArtifactGrant {
    pub(crate) fn response(&self) -> RemoteArtifactGrant {
        RemoteArtifactGrant {
            token: self.token.clone(),
            artifact_id: self.artifact_id.clone(),
            format: self.format.clone(),
            content_state: self.content_state.clone(),
            transfer_sha256: self.transfer_sha256.clone(),
            expires_at_ms: self.expires_at_ms,
        }
    }
}

#[derive(Debug)]
struct ArtifactCandidate {
    artifact_id: String,
    project_name: String,
    format: String,
    path: String,
    digest: String,
    bytes: Vec<u8>,
    protected: bool,
}

#[derive(Debug)]
struct ApprovalCopy {
    title: String,
    body: String,
    send_once: String,
    deny: String,
    details: String,
    details_title: String,
    format_label: String,
    size_label: String,
    project_label: String,
    device_key_label: String,
    source_digest_label: String,
    transfer_digest_label: String,
    close: String,
}

fn translated_value(translations: &serde_json::Value, key: &str, fallback: &str) -> String {
    key.split('.')
        .try_fold(translations, |current, segment| current.get(segment))
        .and_then(serde_json::Value::as_str)
        .unwrap_or(fallback)
        .to_string()
}

fn approval_copy(engine: &PersistenceEngine, file_name: &str, device: &str) -> ApprovalCopy {
    let translations = crate::settings::locale_state_for_engine(engine, None)
        .map(|state| state.translations)
        .unwrap_or_else(|_| serde_json::Value::Null);
    let replace = |value: String| {
        value
            .replace("{name}", file_name)
            .replace("{device}", device)
    };
    let body = replace(translated_value(
        &translations,
        "remote_devices.artifact_transfer.body",
        "OOMU will send the complete file “{name}” to {device}.",
    ));
    let support = translated_value(
        &translations,
        "remote_devices.artifact_transfer.support",
        "The link works once and expires in 10 minutes.",
    );
    ApprovalCopy {
        title: translated_value(
            &translations,
            "remote_devices.artifact_transfer.title",
            "Send this file?",
        ),
        body: format!("{body}\n\n{support}"),
        send_once: translated_value(
            &translations,
            "remote_devices.artifact_transfer.send_once",
            "Send once",
        ),
        deny: translated_value(
            &translations,
            "remote_devices.artifact_transfer.deny",
            "Don’t send",
        ),
        details: translated_value(&translations, "common.details", "Details"),
        details_title: translated_value(
            &translations,
            "remote_devices.artifact_transfer.details_title",
            "Transfer details",
        ),
        format_label: translated_value(
            &translations,
            "remote_devices.artifact_transfer.format",
            "Format",
        ),
        size_label: translated_value(
            &translations,
            "remote_devices.artifact_transfer.size",
            "Size",
        ),
        project_label: translated_value(
            &translations,
            "remote_devices.artifact_transfer.project",
            "Project",
        ),
        device_key_label: translated_value(
            &translations,
            "remote_devices.artifact_transfer.device_key",
            "Device key",
        ),
        source_digest_label: translated_value(
            &translations,
            "remote_devices.artifact_transfer.source_digest",
            "Verified file fingerprint",
        ),
        transfer_digest_label: translated_value(
            &translations,
            "remote_devices.artifact_transfer.transfer_digest",
            "File being sent fingerprint",
        ),
        close: translated_value(&translations, "common.close", "Close"),
    }
}

fn readable_size(bytes: usize) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    let bytes = bytes as f64;
    if bytes >= MIB {
        format!("{:.1} MB", bytes / MIB)
    } else if bytes >= KIB {
        format!("{:.1} KB", bytes / KIB)
    } else {
        format!("{} bytes", bytes as usize)
    }
}

fn resolve_candidate(
    engine: &PersistenceEngine,
    command: &SignedRemoteCommand,
) -> Result<ArtifactCandidate, String> {
    let artifact_id = command
        .payload
        .get("artifactId")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "remote_artifact_required: Choose a file to send.".to_string())?;
    crate::p0_contracts::ArtifactId::parse(artifact_id)?;
    let format = command
        .payload
        .get("format")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("pdf");
    let (path_column, digest_column) =
        match format {
            "pdf" => ("pdf_private_path", "pdf_sha256"),
            "docx" => ("docx_private_path", "docx_sha256"),
            _ => return Err(
                "remote_artifact_format_unsupported: Only verified PDF and Word files can be sent."
                    .to_string(),
            ),
        };
    let query = format!(
        "SELECT r.project_id,p.name,v.{path_column},v.{digest_column},\
         CASE WHEN COALESCE(json_extract(v.verification_json,'$.protected'),0)=1 \
                    OR COALESCE(json_extract(v.provenance_json,'$.protected'),0)=1 \
              THEN 1 ELSE 0 END \
         FROM artifact_records r \
         JOIN projects p ON p.project_id=r.project_id \
         JOIN artifact_versions v ON v.artifact_id=r.artifact_id AND v.version=r.current_version \
         WHERE r.artifact_id=?1 AND v.status='verified'"
    );
    let record = engine
        .open_connection()
        .map_err(|cause| format!("remote_artifact_store_unavailable: {cause}"))?
        .query_row(&query, params![artifact_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)? != 0,
            ))
        })
        .optional()
        .map_err(|cause| format!("remote_artifact_store_read_failed: {cause}"))?
        .ok_or_else(|| {
            "remote_artifact_not_verified: This file is not ready to send.".to_string()
        })?;
    if record.0 != command.project_id {
        return Err(
            "remote_artifact_project_mismatch: This file is outside the device’s Project access."
                .to_string(),
        );
    }
    if record.4 {
        return Err(
            "remote_artifact_protected: This protected file cannot be sent remotely.".to_string(),
        );
    }
    let bytes = std::fs::read(&record.2)
        .map_err(|_| "remote_artifact_missing: This file is no longer on the Mac.".to_string())?;
    let actual_digest = crate::foundation::digest::sha256_hex(&bytes);
    if actual_digest != record.3 {
        return Err(
            "remote_artifact_source_changed: This file changed after OOMU verified it.".to_string(),
        );
    }
    Ok(ArtifactCandidate {
        artifact_id: artifact_id.to_string(),
        project_name: record.1,
        format: format.to_string(),
        path: record.2,
        digest: actual_digest,
        bytes,
        protected: false,
    })
}

fn native_approval(
    engine: &PersistenceEngine,
    candidate: &ArtifactCandidate,
    device: &str,
    command: &SignedRemoteCommand,
) -> bool {
    let name = Path::new(&candidate.path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("file");
    let copy = approval_copy(engine, name, device);
    loop {
        let selected = rfd::MessageDialog::new()
            .set_title(&copy.title)
            .set_description(&copy.body)
            .set_level(rfd::MessageLevel::Warning)
            // The first and default action is the safe choice.
            .set_buttons(rfd::MessageButtons::YesNoCancelCustom(
                copy.deny.clone(),
                copy.send_once.clone(),
                copy.details.clone(),
            ))
            .show();
        if selected == rfd::MessageDialogResult::Custom(copy.send_once.clone()) {
            return true;
        }
        if selected != rfd::MessageDialogResult::Custom(copy.details.clone()) {
            return false;
        }
        let details = format!(
            "{}: {}\n{}: {}\n{}: {}\n{}: {}\n{}: {}\n{}: {}",
            copy.format_label,
            candidate.format.to_uppercase(),
            copy.size_label,
            readable_size(candidate.bytes.len()),
            copy.project_label,
            candidate.project_name,
            copy.device_key_label,
            command.signer_public_key,
            copy.source_digest_label,
            candidate.digest,
            copy.transfer_digest_label,
            candidate.digest,
        );
        rfd::MessageDialog::new()
            .set_title(&copy.details_title)
            .set_description(details)
            .set_level(rfd::MessageLevel::Info)
            .set_buttons(rfd::MessageButtons::OkCustom(copy.close.clone()))
            .show();
    }
}

fn prepare_with_decision(
    identity: &SovereignIdentity,
    command: &SignedRemoteCommand,
    candidate: ArtifactCandidate,
    device_label: &str,
    approved: bool,
) -> Result<PreparedArtifactGrant, String> {
    if !approved {
        return Err("remote_artifact_user_denied: The file stayed on this Mac.".to_string());
    }
    let now = crate::foundation::clock::unix_time_ms_i64();
    let expires_at_ms = now.saturating_add(GRANT_TTL_MS);
    let token = crypto::random_hex(32);
    let receipt_id = crypto::uuid_id("receipt");
    let receipt_payload = serde_json::json!({
        "receiptId": receipt_id,
        "receiptKind": "remote_artifact_approval",
        "commandId": command.command_id,
        "remoteDeviceId": command.remote_device_id,
        "deviceLabel": device_label,
        "deviceKey": command.signer_public_key,
        "projectId": command.project_id,
        "artifactId": candidate.artifact_id,
        "format": candidate.format,
        "bytes": candidate.bytes.len(),
        "contentState": FULL_CONTENT,
        "sourceSha256": candidate.digest,
        "transferSha256": candidate.digest,
        "expiresAtMs": expires_at_ms,
        "approvedAtMs": now,
    })
    .to_string();
    let signed = identity
        .sign_node_payload(&receipt_payload)
        .map_err(|cause| format!("remote_artifact_approval_signing_failed: {}", cause.message))?;
    Ok(PreparedArtifactGrant {
        grant_id: crypto::uuid_id("grant"),
        token_hash: crate::foundation::digest::sha256_hex(token.as_bytes()),
        token,
        remote_device_id: command.remote_device_id.clone(),
        project_id: command.project_id.clone(),
        artifact_id: candidate.artifact_id,
        format: candidate.format,
        source_path: candidate.path.clone(),
        transfer_path: candidate.path,
        source_sha256: candidate.digest.clone(),
        transfer_sha256: candidate.digest,
        content_state: FULL_CONTENT.to_string(),
        protected: candidate.protected,
        expires_at_ms,
        created_at_ms: now,
        approval_receipt: PreparedReceipt {
            receipt_id,
            remote_device_id: command.remote_device_id.clone(),
            command_id: command.command_id.clone(),
            receipt_kind: "remote_artifact_approval".to_string(),
            payload_sha256: signed.payload_hash,
            signer_public_key: signed.public_key,
            signature: signed.signature,
            created_at_ms: now,
        },
    })
}

pub(crate) fn prepare_grant(
    engine: &PersistenceEngine,
    identity: &SovereignIdentity,
    command: &SignedRemoteCommand,
    device_label: &str,
) -> Result<PreparedArtifactGrant, String> {
    let candidate = resolve_candidate(engine, command)?;
    let approved = native_approval(engine, &candidate, device_label, command);
    prepare_with_decision(identity, command, candidate, device_label, approved)
}

pub(crate) fn insert_prepared(
    transaction: &Transaction<'_>,
    grant: &PreparedArtifactGrant,
) -> Result<(), String> {
    let receipt = &grant.approval_receipt;
    transaction
        .execute(
            "INSERT INTO remote_audit_receipts (receipt_id,remote_device_id,command_id,receipt_kind,payload_sha256,signature,created_at_ms,signer_public_key) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                receipt.receipt_id,
                receipt.remote_device_id,
                receipt.command_id,
                receipt.receipt_kind,
                receipt.payload_sha256,
                receipt.signature,
                receipt.created_at_ms,
                receipt.signer_public_key,
            ],
        )
        .map_err(|cause| format!("remote_artifact_approval_receipt_failed: {cause}"))?;
    transaction
        .execute(
            "INSERT INTO remote_artifact_grants (grant_id,token_hash,remote_device_id,project_id,artifact_id,artifact_format,private_path,artifact_sha256,redaction_state,protected,expires_at_ms,created_at_ms,content_state,source_sha256,transfer_sha256,source_path,transfer_path,redaction_manifest_sha256,approval_receipt_id) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,'full_content',?9,?10,?11,?12,?13,?14,?15,?16,NULL,?17)",
            params![
                grant.grant_id,
                grant.token_hash,
                grant.remote_device_id,
                grant.project_id,
                grant.artifact_id,
                grant.format,
                grant.transfer_path,
                grant.transfer_sha256,
                i64::from(grant.protected),
                grant.expires_at_ms,
                grant.created_at_ms,
                grant.content_state,
                grant.source_sha256,
                grant.transfer_sha256,
                grant.source_path,
                grant.transfer_path,
                grant.approval_receipt.receipt_id,
            ],
        )
        .map_err(|cause| format!("remote_artifact_grant_store_failed: {cause}"))?;
    Ok(())
}

pub(crate) fn revalidate_prepared(grant: &PreparedArtifactGrant) -> Result<(), String> {
    let source = std::fs::read(&grant.source_path)
        .map_err(|_| "remote_artifact_missing: This file is no longer on the Mac.".to_string())?;
    let transfer = if grant.source_path == grant.transfer_path {
        source.clone()
    } else {
        std::fs::read(&grant.transfer_path).map_err(|_| {
            "remote_artifact_transfer_missing: The approved file is no longer on the Mac."
                .to_string()
        })?
    };
    if crate::foundation::digest::sha256_hex(&source) != grant.source_sha256 {
        return Err(
            "remote_artifact_source_changed: This file changed after OOMU verified it.".to_string(),
        );
    }
    if crate::foundation::digest::sha256_hex(&transfer) != grant.transfer_sha256 {
        return Err(
            "remote_artifact_transfer_changed: The approved file changed before OOMU could send it."
                .to_string(),
        );
    }
    Ok(())
}

fn associated_data(
    remote_device_id: &str,
    artifact_id: &str,
    format: &str,
    content_state: &str,
    transfer_sha256: &str,
    expires_at_ms: i64,
) -> Vec<u8> {
    serde_json::json!({
        "remoteDeviceId": remote_device_id,
        "artifactId": artifact_id,
        "format": format,
        "contentState": content_state,
        "transferSha256": transfer_sha256,
        "expiresAtMs": expires_at_ms,
    })
    .to_string()
    .into_bytes()
}

pub(crate) fn retrieve(
    engine: &PersistenceEngine,
    request: RetrieveRemoteArtifactRequest,
) -> Result<EncryptedRemoteArtifact, String> {
    if request.token.len() != 64 || hex::decode(&request.token).is_err() {
        return Err("remote_artifact_link_invalid: This file link is invalid.".to_string());
    }
    let now = crate::foundation::clock::unix_time_ms_i64();
    let token_hash = crate::foundation::digest::sha256_hex(request.token.as_bytes());
    let connection = engine
        .open_connection()
        .map_err(|cause| format!("remote_artifact_store_unavailable: {cause}"))?;
    let grant = connection
        .query_row(
            "SELECT g.grant_id,g.artifact_id,g.artifact_format,g.content_state,g.source_path,g.transfer_path,g.source_sha256,g.transfer_sha256,g.expires_at_ms \
             FROM remote_artifact_grants g \
             JOIN remote_devices d ON d.remote_device_id=g.remote_device_id \
             JOIN remote_audit_receipts r ON r.receipt_id=g.approval_receipt_id AND r.receipt_kind='remote_artifact_approval' \
             WHERE g.remote_device_id=?1 AND g.token_hash=?2 AND g.expires_at_ms>=?3 \
               AND g.opened_at_ms IS NULL AND g.revoked_at_ms IS NULL AND g.protected=0 \
               AND g.content_state IN ('full_content','verified_redacted_derivative') \
               AND d.revoked_at_ms IS NULL AND d.expires_at_ms>=?3",
            params![request.remote_device_id, token_hash, now],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, i64>(8)?,
                ))
            },
        )
        .optional()
        .map_err(|cause| format!("remote_artifact_store_read_failed: {cause}"))?
        .ok_or_else(|| {
            "remote_artifact_link_unavailable: This file link expired, was already used, or belongs to another device."
                .to_string()
        })?;
    if grant.3 == FULL_CONTENT && (grant.4 != grant.5 || grant.6 != grant.7) {
        return Err(
            "remote_artifact_declaration_mismatch: OOMU stopped a file whose transfer declaration did not match."
                .to_string(),
        );
    }
    let source_bytes = std::fs::read(&grant.4)
        .map_err(|_| "remote_artifact_missing: This file is no longer on the Mac.".to_string())?;
    let transfer_bytes = if grant.4 == grant.5 {
        source_bytes.clone()
    } else {
        std::fs::read(&grant.5).map_err(|_| {
            "remote_artifact_transfer_missing: The approved file is no longer on the Mac."
                .to_string()
        })?
    };
    if crate::foundation::digest::sha256_hex(&source_bytes) != grant.6 {
        return Err(
            "remote_artifact_source_changed: This file changed after OOMU verified it.".to_string(),
        );
    }
    if crate::foundation::digest::sha256_hex(&transfer_bytes) != grant.7 {
        return Err(
            "remote_artifact_transfer_changed: The approved file no longer matches its transfer receipt."
                .to_string(),
        );
    }
    let aad = associated_data(
        &request.remote_device_id,
        &grant.1,
        &grant.2,
        &grant.3,
        &grant.7,
        grant.8,
    );
    let key_material = Sha256::digest(request.token.as_bytes());
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key_material));
    let nonce = ChaCha20Poly1305::generate_nonce(&mut AeadOsRng);
    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: transfer_bytes.as_slice(),
                aad: aad.as_slice(),
            },
        )
        .map_err(|_| {
            "remote_artifact_encryption_failed: The file could not be secured.".to_string()
        })?;

    let transaction =
        rusqlite::Transaction::new_unchecked(&connection, rusqlite::TransactionBehavior::Immediate)
            .map_err(|cause| format!("remote_artifact_consume_failed: {cause}"))?;
    let consumed = transaction
        .execute(
            "UPDATE remote_artifact_grants SET opened_at_ms=?4 WHERE grant_id=?1 AND remote_device_id=?2 AND token_hash=?3 AND opened_at_ms IS NULL AND revoked_at_ms IS NULL AND expires_at_ms>=?4",
            params![grant.0, request.remote_device_id, token_hash, now],
        )
        .map_err(|cause| format!("remote_artifact_consume_failed: {cause}"))?;
    if consumed != 1 {
        return Err("remote_artifact_link_used: This file link was already used.".to_string());
    }
    transaction
        .commit()
        .map_err(|cause| format!("remote_artifact_consume_failed: {cause}"))?;
    Ok(EncryptedRemoteArtifact {
        artifact_id: grant.1,
        format: grant.2,
        nonce_base64: STANDARD.encode(nonce),
        ciphertext_sha256: crate::foundation::digest::sha256_hex(&ciphertext),
        ciphertext_base64: STANDARD.encode(ciphertext),
        source_sha256: grant.6,
        transfer_sha256: grant.7,
        content_state: grant.3,
        expires_at_ms: grant.8,
        associated_data_base64: STANDARD.encode(aad),
    })
}

#[cfg(test)]
pub(crate) fn prepare_grant_for_test(
    engine: &PersistenceEngine,
    identity: &SovereignIdentity,
    command: &SignedRemoteCommand,
    device_label: &str,
    approved: bool,
) -> Result<PreparedArtifactGrant, String> {
    let candidate = resolve_candidate(engine, command)?;
    prepare_with_decision(identity, command, candidate, device_label, approved)
}
