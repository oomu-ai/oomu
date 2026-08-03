use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use oomu_lib::foundation::digest::sha256_hex;
use serde::Deserialize;
use std::{env, fs, path::PathBuf};

#[derive(Debug, Deserialize)]
struct AirlockEnvelope {
    algorithm: String,
    aead: String,
    artifact_id: String,
    mission_id: String,
    ciphertext_b64: String,
    ciphertext_sha256: String,
    signed_payload: String,
    signature: SignatureBlock,
    finality_checksum: String,
    exported_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct SignatureBlock {
    public_key: String,
    signature: String,
    payload_hash: String,
}

fn main() {
    let mut args = env::args().skip(1);
    let Some(envelope_path) = args.next().map(PathBuf::from) else {
        eprintln!("usage: ark_verify <artifact.oomu-airlock.json> [finality_checksum]");
        std::process::exit(2);
    };
    let expected_checksum = args.next();

    match verify_envelope(&envelope_path, expected_checksum.as_deref()) {
        Ok(message) => println!("{message}"),
        Err(error) => {
            eprintln!("Ark verification failed: {error}");
            std::process::exit(1);
        }
    }
}

fn verify_envelope(path: &PathBuf, expected_checksum: Option<&str>) -> Result<String, String> {
    let envelope_text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let envelope: AirlockEnvelope =
        serde_json::from_str(&envelope_text).map_err(|error| error.to_string())?;
    if envelope.algorithm != "ML-KEM-768" || envelope.aead != "ChaCha20Poly1305" {
        return Err("Unsupported Airlock envelope algorithm.".to_string());
    }
    let ciphertext = BASE64
        .decode(&envelope.ciphertext_b64)
        .map_err(|error| error.to_string())?;
    if envelope.ciphertext_sha256 != sha256_hex(&ciphertext) {
        return Err("Airlock ciphertext checksum mismatch.".to_string());
    }
    let finality = sha256_hex(
        finality_payload(
            &envelope.artifact_id,
            &envelope.mission_id,
            &envelope.signature.payload_hash,
            &envelope.ciphertext_sha256,
            envelope.exported_at_ms,
        )
        .as_bytes(),
    );
    if finality != envelope.finality_checksum {
        return Err("Airlock finality checksum mismatch.".to_string());
    }
    if let Some(expected) = expected_checksum {
        if expected.trim() != finality {
            return Err("Provided Finality Checksum does not match envelope.".to_string());
        }
    }
    verify_signature(&envelope.signed_payload, &envelope.signature)?;
    Ok(format!(
        "Ark offline verification passed: mission={} finality_checksum={}",
        envelope.mission_id, envelope.finality_checksum
    ))
}

fn verify_signature(payload: &str, signature: &SignatureBlock) -> Result<(), String> {
    if sha256_hex(payload.as_bytes()) != signature.payload_hash {
        return Err("Airlock signature payload hash mismatch.".to_string());
    }
    let public_key_bytes = hex::decode(&signature.public_key).map_err(|error| error.to_string())?;
    let public_key_array: [u8; 32] = public_key_bytes
        .try_into()
        .map_err(|_| "Invalid Ed25519 public key length.".to_string())?;
    let verifying_key =
        VerifyingKey::from_bytes(&public_key_array).map_err(|error| error.to_string())?;
    let signature_bytes = hex::decode(&signature.signature).map_err(|error| error.to_string())?;
    let ed_signature =
        Signature::from_slice(&signature_bytes).map_err(|error| error.to_string())?;
    verifying_key
        .verify(payload.as_bytes(), &ed_signature)
        .map_err(|_| "Airlock signature verification failed.".to_string())
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
    use ed25519_dalek::{Signer, SigningKey};
    use rand_core::OsRng;

    #[test]
    fn verifies_mock_offline_airlock_envelope() {
        let signing_key = SigningKey::generate(&mut OsRng);
        let ciphertext = b"offline ciphertext bytes";
        let signed_payload = serde_json::json!({
            "algorithm": "ML-KEM-768+ChaCha20Poly1305",
            "artifact_id": "mock-mission",
            "mission_id": "mock-mission",
            "source_artifact_sha256": sha256_hex(b"plain"),
            "ciphertext_sha256": sha256_hex(ciphertext),
        })
        .to_string();
        let signature = signing_key.sign(signed_payload.as_bytes());
        let payload_hash = sha256_hex(signed_payload.as_bytes());
        let finality_checksum = sha256_hex(
            finality_payload(
                "mock-mission",
                "mock-mission",
                &payload_hash,
                &sha256_hex(ciphertext),
                42,
            )
            .as_bytes(),
        );
        let envelope = serde_json::json!({
            "algorithm": "ML-KEM-768",
            "aead": "ChaCha20Poly1305",
            "artifact_id": "mock-mission",
            "mission_id": "mock-mission",
            "ciphertext_b64": BASE64.encode(ciphertext),
            "ciphertext_sha256": sha256_hex(ciphertext),
            "signed_payload": signed_payload,
            "signature": {
                "public_key": hex::encode(signing_key.verifying_key().to_bytes()),
                "signature": hex::encode(signature.to_bytes()),
                "payload_hash": payload_hash,
            },
            "finality_checksum": finality_checksum,
            "exported_at_ms": 42,
        });
        let path = env::temp_dir().join("oomu-mock-airlock-envelope.json");
        fs::write(&path, serde_json::to_string_pretty(&envelope).unwrap()).unwrap();

        let result = verify_envelope(&path, Some(&finality_checksum)).unwrap();
        assert!(result.contains("Ark offline verification passed"));
        let _ = fs::remove_file(path);
    }
}
