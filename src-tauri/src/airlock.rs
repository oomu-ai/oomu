use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chacha20poly1305::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    ChaCha20Poly1305, Key,
};
use pqcrypto_mlkem::mlkem768;
use pqcrypto_traits::kem::{
    Ciphertext as KemCiphertext, PublicKey as KemPublicKey, SharedSecret as KemSharedSecret,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use crate::{
    foundation::{clock::unix_time_ms_i64 as unix_time_ms, digest::sha256_hex},
    shield_gate::{AirlockExportRequest, CommandStatus, ExecuteCommandResponse},
    sovereign_identity::{SignatureBlock, SovereignIdentity},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AirlockEnvelope {
    pub version: String,
    pub algorithm: String,
    pub aead: String,
    pub artifact_id: String,
    pub mission_id: String,
    pub source_artifact: String,
    pub public_key_b64: String,
    pub encapsulated_key_b64: String,
    pub nonce_b64: String,
    pub ciphertext_b64: String,
    pub ciphertext_sha256: String,
    pub signed_payload: String,
    pub signature: SignatureBlock,
    pub finality_checksum: String,
    pub exported_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AirlockExportResult {
    pub envelope_path: String,
    pub checksum_path: String,
    pub finality_checksum: String,
    pub pqc_algorithm: String,
    pub local_decryption_verified: bool,
}

#[derive(Debug, Clone)]
pub struct Airlock {
    project_root: PathBuf,
}

impl Airlock {
    pub fn new(project_root: PathBuf) -> Self {
        Self { project_root }
    }

    pub fn export_sync(
        &self,
        request: AirlockExportRequest,
        identity: &SovereignIdentity,
    ) -> Result<ExecuteCommandResponse, String> {
        let source = self.guard_artifact_path(&request.artifact_path)?;
        let mount = self.guard_mount_path(&request.mount_path)?;
        fs::create_dir_all(&mount).map_err(|error| error.to_string())?;

        let artifact_bytes = fs::read(&source).map_err(|error| error.to_string())?;
        let artifact_id = request.mission_id.trim().replace(
            |character: char| !character.is_ascii_alphanumeric() && character != '-',
            "_",
        );
        let result = self.write_pqc_envelope(
            &artifact_id,
            request.mission_id.trim(),
            &source,
            &mount,
            &artifact_bytes,
            identity,
        )?;

        Ok(ExecuteCommandResponse {
            operation: "airlock_export".to_string(),
            status: CommandStatus::Completed,
            message: format!(
                "Airlock export complete: ML-KEM-768 wrapped artifact written to {} with finality checksum {}.",
                result.envelope_path, result.finality_checksum
            ),
            metrics: None,
            claims: vec![
                "CLAIM operation=airlock_export status=completed".to_string(),
                format!("CLAIM pqc_algorithm={}", result.pqc_algorithm),
                format!(
                    "CLAIM local_decryption_verified={}",
                    result.local_decryption_verified
                ),
                format!("CLAIM finality_checksum={}", result.finality_checksum),
            ],
            verified: result.local_decryption_verified,
            model_used: None,
        })
    }

    fn write_pqc_envelope(
        &self,
        artifact_id: &str,
        mission_id: &str,
        source: &Path,
        mount: &Path,
        artifact_bytes: &[u8],
        identity: &SovereignIdentity,
    ) -> Result<AirlockExportResult, String> {
        let (public_key, secret_key) = mlkem768::keypair();
        let (shared_secret, encapsulated_key) = mlkem768::encapsulate(&public_key);
        let key_material = Sha256::digest(shared_secret.as_bytes());
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&key_material));
        let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
        let ciphertext = cipher
            .encrypt(&nonce, artifact_bytes)
            .map_err(|error| format!("PQC payload encryption failed: {error}"))?;

        let decrypted = {
            let decapsulated = mlkem768::decapsulate(&encapsulated_key, &secret_key);
            let decrypt_key_material = Sha256::digest(decapsulated.as_bytes());
            let decrypt_cipher = ChaCha20Poly1305::new(Key::from_slice(&decrypt_key_material));
            decrypt_cipher
                .decrypt(&nonce, ciphertext.as_slice())
                .map_err(|error| format!("PQC local decrypt verification failed: {error}"))?
        };
        if decrypted != artifact_bytes {
            return Err("PQC local decrypt verification failed: plaintext mismatch.".to_string());
        }

        let signed_payload = serde_json::json!({
            "algorithm": "ML-KEM-768+ChaCha20Poly1305",
            "artifact_id": artifact_id,
            "mission_id": mission_id,
            "source_artifact_sha256": sha256_hex(artifact_bytes),
            "ciphertext_sha256": sha256_hex(&ciphertext),
        })
        .to_string();
        let signature = identity
            .sign_payload(&signed_payload)
            .map_err(|error| error.message)?;
        let exported_at_ms = unix_time_ms();
        let finality_payload = finality_payload(
            artifact_id,
            mission_id,
            &signature.payload_hash,
            &sha256_hex(&ciphertext),
            exported_at_ms,
        );
        let finality_checksum = sha256_hex(finality_payload.as_bytes());
        let envelope = AirlockEnvelope {
            version: "oomu-airlock-v1".to_string(),
            algorithm: "ML-KEM-768".to_string(),
            aead: "ChaCha20Poly1305".to_string(),
            artifact_id: artifact_id.to_string(),
            mission_id: mission_id.to_string(),
            source_artifact: source.display().to_string(),
            public_key_b64: BASE64.encode(public_key.as_bytes()),
            encapsulated_key_b64: BASE64.encode(encapsulated_key.as_bytes()),
            nonce_b64: BASE64.encode(nonce),
            ciphertext_b64: BASE64.encode(&ciphertext),
            ciphertext_sha256: sha256_hex(&ciphertext),
            signed_payload,
            signature,
            finality_checksum: finality_checksum.clone(),
            exported_at_ms,
        };

        let envelope_path = mount.join(format!("{artifact_id}.oomu-airlock.json"));
        let checksum_path = mount.join(format!("{artifact_id}.finality.sha256"));
        let envelope_json =
            serde_json::to_string_pretty(&envelope).map_err(|error| error.to_string())?;
        fs::write(&envelope_path, envelope_json).map_err(|error| error.to_string())?;
        fs::write(
            &checksum_path,
            format!("{finality_checksum}  {}\n", envelope_path.display()),
        )
        .map_err(|error| error.to_string())?;

        Ok(AirlockExportResult {
            envelope_path: envelope_path.to_string_lossy().to_string(),
            checksum_path: checksum_path.to_string_lossy().to_string(),
            finality_checksum,
            pqc_algorithm: "ML-KEM-768+ChaCha20Poly1305".to_string(),
            local_decryption_verified: true,
        })
    }

    fn guard_artifact_path(&self, requested: &str) -> Result<PathBuf, String> {
        let path = self.resolve_under_project(requested)?;
        if !path.starts_with(self.project_root.join("ark")) {
            return Err(format!(
                "airlock_export rejected non-Ark artifact path: {}",
                path.display()
            ));
        }
        Ok(path)
    }

    fn guard_mount_path(&self, requested: &str) -> Result<PathBuf, String> {
        let requested_path = Path::new(requested);
        if requested_path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        {
            return Err("airlock_export rejected mount path traversal.".to_string());
        }
        let path = if requested_path.is_absolute() {
            requested_path.to_path_buf()
        } else {
            self.project_root.join(requested_path)
        };
        let is_real_mount = if cfg!(target_os = "macos") {
            path.starts_with("/Volumes")
        } else if cfg!(target_os = "linux") {
            path.starts_with("/media")
        } else {
            path.starts_with("/Volumes") || path.starts_with("/media")
        };

        if is_real_mount {
            Ok(path)
        } else {
            Err(format!(
                "airlock_export rejected insecure mount point: {}. Use a real system mount (/Volumes on macOS or /media on Linux).",
                path.display()
            ))
        }
    }

    fn resolve_under_project(&self, requested: &str) -> Result<PathBuf, String> {
        let requested_path = Path::new(requested);
        if requested_path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        {
            return Err("airlock_export rejected path traversal.".to_string());
        }
        let path = if requested_path.is_absolute() {
            requested_path.to_path_buf()
        } else {
            self.project_root.join(requested_path)
        };
        if path.starts_with(&self.project_root) {
            Ok(path)
        } else {
            Err(format!(
                "airlock_export rejected path outside project quarantine: {}",
                path.display()
            ))
        }
    }
}

fn finality_payload(
    artifact_id: &str,
    mission_id: &str,
    signature_payload_hash: &str,
    ciphertext_sha256: &str,
    exported_at_ms: i64,
) -> String {
    serde_json::json!({
        "artifact_id": artifact_id,
        "mission_id": mission_id,
        "signature_payload_hash": signature_payload_hash,
        "ciphertext_sha256": ciphertext_sha256,
        "exported_at_ms": exported_at_ms,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mlkem_wrapping_round_trips_locally() {
        let plaintext = b"oomu ark q-day shield";
        let (public_key, secret_key) = mlkem768::keypair();
        let (shared_secret, encapsulated_key) = mlkem768::encapsulate(&public_key);
        let key_material = Sha256::digest(shared_secret.as_bytes());
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&key_material));
        let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
        let ciphertext = cipher.encrypt(&nonce, plaintext.as_ref()).unwrap();

        let decapsulated = mlkem768::decapsulate(&encapsulated_key, &secret_key);
        let decrypt_key_material = Sha256::digest(decapsulated.as_bytes());
        let decrypt_cipher = ChaCha20Poly1305::new(Key::from_slice(&decrypt_key_material));
        let decrypted = decrypt_cipher
            .decrypt(&nonce, ciphertext.as_slice())
            .unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_airlock_guard_mount_path_enforcement() {
        let airlock = Airlock::new(PathBuf::from("/tmp/oomu-project"));

        // 1. Real system mount on macOS/Linux should be accepted
        #[cfg(target_os = "macos")]
        {
            let res = airlock.guard_mount_path("/Volumes/MyExternalDrive");
            assert!(
                res.is_ok(),
                "Real system mount /Volumes should be accepted: {:?}",
                res
            );
        }
        #[cfg(target_os = "linux")]
        {
            let res = airlock.guard_mount_path("/media/usb_drive");
            assert!(
                res.is_ok(),
                "Real system mount /media should be accepted: {:?}",
                res
            );
        }

        // 2. Project-local paths cannot imitate removable media in any profile.
        let res_local = airlock.guard_mount_path("/tmp/oomu-project/airlock_exports/mission_1");
        assert!(
            res_local.is_err(),
            "Project-local paths must not satisfy the removable-media boundary: {:?}",
            res_local
        );

        // 3. Insecure random paths should be rejected
        let res_insecure = airlock.guard_mount_path("/tmp/insecure_random_path");
        assert!(
            res_insecure.is_err(),
            "Insecure paths must be rejected: {:?}",
            res_insecure
        );
    }
}
