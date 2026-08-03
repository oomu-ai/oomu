use super::mod_package::{relative_archive_path, ArchiveEntry, MAX_MOD_ARCHIVE_SIZE};
use ed25519_dalek::{Signature, VerifyingKey};
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::Path,
    sync::{Mutex, MutexGuard, OnceLock},
};

// This is the existing reviewed offline Architect Root. Mod and registry signatures use
// distinct domain-separated messages so a signature from one protocol cannot cross into another.
// Rotation requires a reviewed source change; the matching private key is never shipped.
pub(crate) const ELDRIS_REVIEW_PUBLIC_KEY: [u8; 32] = [
    0xd4, 0x07, 0x13, 0xa6, 0x7f, 0x6e, 0xc7, 0x3f, 0x2c, 0xad, 0xfa, 0x89, 0xbb, 0xc9, 0x2d, 0x45,
    0x35, 0x05, 0x56, 0x55, 0xd3, 0x68, 0xcc, 0x06, 0x06, 0x05, 0x1b, 0x6b, 0x60, 0xf2, 0x96, 0x20,
];
const REVIEW_KEY_ID: &str = "eldris-mod-review-v1";
const MOD_PAYLOAD_DOMAIN: &[u8] = b"OOMU-MOD-PAYLOAD-V1";
const MOD_REVIEW_DOMAIN: &[u8] = b"OOMU-MOD-REVIEW-V1";
const MOD_PUBLISHER_DOMAIN: &[u8] = b"OOMU-MOD-PUBLISHER-V1";
const REGISTRY_DOMAIN: &[u8] = b"OOMU-CAPABILITY-REGISTRY-V1";
const DETACHED_REVIEW_SIGNATURE_PATH: &str = "signature.sig";
const MAX_INSTALLED_FILES: usize = 256;
const MAX_INSTALLED_ENTRIES: usize = 512;
static MOD_PACKAGE_OPERATION_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub(crate) fn lock_mod_package_operation() -> Result<MutexGuard<'static, ()>, String> {
    MOD_PACKAGE_OPERATION_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| "Mod verification is temporarily unavailable.".to_string())
}

pub(crate) const INSTALLED_MODS_SCHEMA: &str = "
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS installed_mods (
    id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT NOT NULL,
    is_active INTEGER NOT NULL DEFAULT 0, version TEXT NOT NULL, author TEXT NOT NULL,
    category TEXT NOT NULL, package_size TEXT NOT NULL, last_updated TEXT NOT NULL,
    permissions_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(permissions_json)),
    endpoints_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(endpoints_json)),
    installed_path TEXT NOT NULL, manifest_json TEXT NOT NULL CHECK (json_valid(manifest_json)),
    default_system_prompt TEXT, entrypoint TEXT NOT NULL,
    review_state TEXT NOT NULL DEFAULT 'unreviewed' CHECK(review_state IN ('reviewed','unreviewed','revoked')),
    publisher_identity_verified INTEGER NOT NULL DEFAULT 0,
    integrity_state TEXT NOT NULL DEFAULT 'unsigned' CHECK(integrity_state IN ('verified','unsigned','modified')),
    payload_sha256 TEXT NOT NULL DEFAULT '', is_built_in INTEGER NOT NULL DEFAULT 0,
    installed_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_installed_mods_active
ON installed_mods(is_active, name COLLATE NOCASE);
";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ModTrust {
    pub(crate) review_state: &'static str,
    pub(crate) publisher_identity_verified: bool,
    pub(crate) integrity_state: &'static str,
    pub(crate) payload_sha256: String,
}

pub(crate) struct InstalledModTrustEvaluation {
    pub(crate) mod_id: String,
    pub(crate) version: String,
    pub(crate) manifest: Value,
    pub(crate) trust: ModTrust,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReviewSignatureEnvelope {
    algorithm: String,
    key_id: String,
    payload_sha256: String,
    signature: String,
    #[serde(default)]
    signed_at: Option<String>,
}

pub(crate) fn ensure_installed_mod_trust_columns(connection: &Connection) -> Result<(), String> {
    for (column, ddl) in [
        (
            "review_state",
            "ALTER TABLE installed_mods ADD COLUMN review_state TEXT NOT NULL DEFAULT 'unreviewed' CHECK(review_state IN ('reviewed','unreviewed','revoked'))",
        ),
        (
            "publisher_identity_verified",
            "ALTER TABLE installed_mods ADD COLUMN publisher_identity_verified INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "integrity_state",
            "ALTER TABLE installed_mods ADD COLUMN integrity_state TEXT NOT NULL DEFAULT 'unsigned' CHECK(integrity_state IN ('verified','unsigned','modified'))",
        ),
        (
            "payload_sha256",
            "ALTER TABLE installed_mods ADD COLUMN payload_sha256 TEXT NOT NULL DEFAULT ''",
        ),
        (
            "is_built_in",
            "ALTER TABLE installed_mods ADD COLUMN is_built_in INTEGER NOT NULL DEFAULT 0",
        ),
    ] {
        if !table_has_column(connection, "installed_mods", column)? {
            connection.execute(ddl, []).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn table_has_column(connection: &Connection, table: &str, column: &str) -> Result<bool, String> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|error| error.to_string())?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    Ok(columns.iter().any(|candidate| candidate == column))
}

pub(super) fn evaluate_package(
    manifest: &Value,
    entries: &[ArchiveEntry],
) -> Result<ModTrust, String> {
    let key = production_review_key()?;
    evaluate_package_with_key(manifest, entries, &key)
}

fn evaluate_package_with_key(
    manifest: &Value,
    entries: &[ArchiveEntry],
    review_key: &VerifyingKey,
) -> Result<ModTrust, String> {
    let actual_digest = canonical_payload_digest(entries)?;
    let actual_hex = hex::encode(actual_digest);
    let Some(bundle) = manifest.get("capability_bundle").and_then(Value::as_object) else {
        return Ok(unsigned_trust(actual_hex));
    };
    let embedded = bundle.get("reviewSignature");
    let detached = match entries
        .iter()
        .find(|entry| entry.name == DETACHED_REVIEW_SIGNATURE_PATH)
    {
        Some(entry) => match serde_json::from_slice::<Value>(&entry.bytes) {
            Ok(value) => Some(value),
            Err(_) => return Ok(modified_trust(actual_hex)),
        },
        None => None,
    };
    if embedded.is_some() && detached.is_some() {
        return Ok(modified_trust(actual_hex));
    }
    let review_value = embedded.cloned().or(detached);
    let declared_digest = bundle
        .get("payloadSha256")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            review_value
                .as_ref()
                .and_then(|value| value.get("payloadSha256"))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
        });
    let has_signature_material = bundle.get("signature").is_some() || review_value.is_some();
    if declared_digest.is_none() && !has_signature_material {
        return Ok(unsigned_trust(actual_hex));
    }
    let digest_matches = declared_digest.is_some_and(|value| {
        value == actual_hex
            && value.len() == 64
            && value.bytes().all(|byte| byte.is_ascii_hexdigit())
    });
    if !digest_matches {
        return Ok(modified_trust(actual_hex));
    }

    let publisher_key = bundle
        .get("publisher")
        .and_then(Value::as_object)
        .and_then(|publisher| publisher.get("publicKey"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty());
    let publisher_signature = bundle
        .get("signature")
        .and_then(|value| {
            value.as_str().or_else(|| {
                value
                    .as_object()
                    .filter(|signature| {
                        signature.get("algorithm").and_then(Value::as_str) == Some("ed25519")
                            && signature.get("payloadSha256").and_then(Value::as_str)
                                == Some(actual_hex.as_str())
                    })
                    .and_then(|signature| signature.get("signature"))
                    .and_then(Value::as_str)
            })
        })
        .filter(|value| !value.is_empty());
    let publisher_present = publisher_key.is_some() || publisher_signature.is_some();
    let publisher_identity_verified = match (publisher_key, publisher_signature) {
        (Some(key), Some(signature)) => {
            verify_hex_key_signature(key, MOD_PUBLISHER_DOMAIN, &actual_digest, signature)
        }
        _ => false,
    };

    let Some(review_value) = review_value else {
        return Ok(ModTrust {
            review_state: "unreviewed",
            publisher_identity_verified,
            integrity_state: if publisher_identity_verified {
                "verified"
            } else if publisher_present {
                "modified"
            } else {
                "unsigned"
            },
            payload_sha256: actual_hex,
        });
    };
    let envelope = match serde_json::from_value::<ReviewSignatureEnvelope>(review_value) {
        Ok(envelope) => envelope,
        Err(_) => return Ok(modified_trust(actual_hex)),
    };
    let _ = envelope.signed_at.as_deref();
    let reviewed = envelope.algorithm == "ed25519"
        && envelope.key_id == REVIEW_KEY_ID
        && envelope.payload_sha256 == actual_hex
        && verify_with_key(
            review_key,
            MOD_REVIEW_DOMAIN,
            &actual_digest,
            &envelope.signature,
        );
    if !reviewed {
        return Ok(ModTrust {
            review_state: "unreviewed",
            publisher_identity_verified,
            integrity_state: "modified",
            payload_sha256: actual_hex,
        });
    }
    Ok(ModTrust {
        review_state: "reviewed",
        publisher_identity_verified,
        integrity_state: "verified",
        payload_sha256: actual_hex,
    })
}

pub(super) fn evaluate_installed_directory(
    root: &Path,
) -> Result<InstalledModTrustEvaluation, String> {
    let entries = collect_installed_entries(root)?;
    let manifest_entry = entries
        .iter()
        .find(|entry| entry.name == "manifest.json")
        .ok_or_else(|| "Installed mod is missing manifest.json.".to_string())?;
    let manifest = serde_json::from_slice::<Value>(&manifest_entry.bytes)
        .map_err(|_| "Installed mod manifest is unreadable.".to_string())?;
    let mod_id = manifest
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "Installed mod identity is unreadable.".to_string())?
        .trim()
        .to_string();
    let version = manifest
        .get("version")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "Installed mod version is unreadable.".to_string())?
        .trim()
        .to_string();
    Ok(InstalledModTrustEvaluation {
        mod_id,
        version,
        manifest: manifest.clone(),
        trust: evaluate_package(&manifest, &entries)?,
    })
}

fn collect_installed_entries(root: &Path) -> Result<Vec<ArchiveEntry>, String> {
    let root =
        fs::canonicalize(root).map_err(|_| "Installed mod files are unavailable.".to_string())?;
    let mut pending = vec![root.clone()];
    let mut entries = Vec::new();
    let mut total = 0_u64;
    let mut visited = 0_usize;
    while let Some(directory) = pending.pop() {
        let mut children = fs::read_dir(&directory)
            .map_err(|_| "Installed mod files are unavailable.".to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "Installed mod files are unavailable.".to_string())?;
        children.sort_by_key(|entry| entry.file_name());
        for child in children {
            visited += 1;
            if visited > MAX_INSTALLED_ENTRIES {
                return Err("Installed mod contains too many files or folders.".to_string());
            }
            let metadata = fs::symlink_metadata(child.path())
                .map_err(|_| "Installed mod files are unavailable.".to_string())?;
            if metadata.file_type().is_symlink() {
                return Err("Installed mod contains an unsupported file link.".to_string());
            }
            if metadata.is_dir() {
                pending.push(child.path());
                continue;
            }
            if !metadata.is_file() || entries.len() >= MAX_INSTALLED_FILES {
                return Err("Installed mod contains unsupported files.".to_string());
            }
            if total.saturating_add(metadata.len()) > MAX_MOD_ARCHIVE_SIZE {
                return Err("Installed mod is larger than the supported limit.".to_string());
            }
            let relative = child
                .path()
                .strip_prefix(&root)
                .map_err(|_| "Installed mod path is invalid.".to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            relative_archive_path(&relative)?;
            let mut bytes = Vec::with_capacity(metadata.len() as usize);
            fs::File::open(child.path())
                .map_err(|_| "Installed mod files are unavailable.".to_string())?
                .take(MAX_MOD_ARCHIVE_SIZE.saturating_sub(total) + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| "Installed mod files are unavailable.".to_string())?;
            if bytes.len() as u64 != metadata.len() {
                return Err("Installed mod changed while it was being verified.".to_string());
            }
            total = total.saturating_add(bytes.len() as u64);
            if total > MAX_MOD_ARCHIVE_SIZE {
                return Err("Installed mod is larger than the supported limit.".to_string());
            }
            entries.push(ArchiveEntry {
                name: relative,
                bytes,
            });
        }
    }
    ensure_unique_paths(entries.iter().map(|entry| entry.name.as_str()))?;
    Ok(entries)
}

fn canonical_payload_digest(entries: &[ArchiveEntry]) -> Result<[u8; 32], String> {
    ensure_unique_paths(entries.iter().map(|entry| entry.name.as_str()))?;
    let mut ordered = entries
        .iter()
        .filter(|entry| entry.name != DETACHED_REVIEW_SIGNATURE_PATH)
        .collect::<Vec<_>>();
    ordered.sort_by(|left, right| left.name.cmp(&right.name));
    let mut hasher = Sha256::new();
    hasher.update(MOD_PAYLOAD_DOMAIN);
    hasher.update([0]);
    for entry in ordered {
        let payload = if entry.name == "manifest.json" {
            canonical_unsigned_manifest(&entry.bytes)?
        } else {
            entry.bytes.clone()
        };
        hasher.update((entry.name.len() as u64).to_be_bytes());
        hasher.update(entry.name.as_bytes());
        hasher.update((payload.len() as u64).to_be_bytes());
        hasher.update(&payload);
    }
    Ok(hasher.finalize().into())
}

fn ensure_unique_paths<'a>(paths: impl IntoIterator<Item = &'a str>) -> Result<(), String> {
    let mut seen = HashSet::new();
    for path in paths {
        if !path.is_ascii() || !seen.insert(path.to_ascii_lowercase()) {
            return Err("Mod package contains ambiguous file paths.".to_string());
        }
    }
    Ok(())
}

fn canonical_unsigned_manifest(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut manifest = serde_json::from_slice::<Value>(bytes)
        .map_err(|_| "Mod package manifest is unreadable.".to_string())?;
    if let Some(bundle) = manifest
        .get_mut("capability_bundle")
        .and_then(Value::as_object_mut)
    {
        bundle.remove("payloadSha256");
        bundle.remove("signature");
        bundle.remove("reviewSignature");
    }
    serde_json::to_vec(&canonical_json(manifest)).map_err(|error| error.to_string())
}

fn canonical_json(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(canonical_json).collect()),
        Value::Object(values) => {
            let mut pairs = values.into_iter().collect::<Vec<_>>();
            pairs.sort_by(|left, right| left.0.cmp(&right.0));
            Value::Object(
                pairs
                    .into_iter()
                    .map(|(key, value)| (key, canonical_json(value)))
                    .collect::<Map<_, _>>(),
            )
        }
        scalar => scalar,
    }
}

fn unsigned_trust(payload_sha256: String) -> ModTrust {
    ModTrust {
        review_state: "unreviewed",
        publisher_identity_verified: false,
        integrity_state: "unsigned",
        payload_sha256,
    }
}

fn modified_trust(payload_sha256: String) -> ModTrust {
    ModTrust {
        review_state: "unreviewed",
        publisher_identity_verified: false,
        integrity_state: "modified",
        payload_sha256,
    }
}

fn production_review_key() -> Result<VerifyingKey, String> {
    VerifyingKey::from_bytes(&ELDRIS_REVIEW_PUBLIC_KEY)
        .map_err(|_| "The built-in mod trust key is unavailable.".to_string())
}

fn verify_hex_key_signature(
    public_key: &str,
    domain: &[u8],
    digest: &[u8; 32],
    signature: &str,
) -> bool {
    let Ok(key_bytes) = hex::decode(public_key) else {
        return false;
    };
    let Ok(key_bytes) = <[u8; 32]>::try_from(key_bytes) else {
        return false;
    };
    let Ok(key) = VerifyingKey::from_bytes(&key_bytes) else {
        return false;
    };
    verify_with_key(&key, domain, digest, signature)
}

fn verify_with_key(key: &VerifyingKey, domain: &[u8], digest: &[u8; 32], signature: &str) -> bool {
    let Ok(signature_bytes) = hex::decode(signature) else {
        return false;
    };
    let Ok(signature) = Signature::from_slice(&signature_bytes) else {
        return false;
    };
    key.verify_strict(&signature_message(domain, digest), &signature)
        .is_ok()
}

fn signature_message(domain: &[u8], digest: &[u8; 32]) -> Vec<u8> {
    let mut message = Vec::with_capacity(domain.len() + 1 + digest.len());
    message.extend_from_slice(domain);
    message.push(0);
    message.extend_from_slice(digest);
    message
}

pub(crate) fn verify_registry_catalog(
    supplied_public_key: &str,
    payload: &[u8],
    signature: &str,
) -> Result<(), String> {
    let supplied = hex::decode(supplied_public_key)
        .map_err(|_| "The registry trust information is invalid.".to_string())?;
    if supplied.as_slice() != ELDRIS_REVIEW_PUBLIC_KEY {
        return Err("The registry is not signed by Eldris.".to_string());
    }
    let digest: [u8; 32] = Sha256::digest(payload).into();
    let key = production_review_key()?;
    if !verify_with_key(&key, REGISTRY_DOMAIN, &digest, signature) {
        return Err("The registry signature could not be verified.".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use rand_core::OsRng;
    use rusqlite::Connection;
    use serde_json::json;

    fn unsigned_entries() -> (Value, Vec<ArchiveEntry>) {
        let manifest = json!({
            "id": "com.example.mod",
            "name": "Example",
            "version": "1.0.0",
            "author": "Acme",
            "description": "Example mod",
            "entrypoint": "index.js",
            "capability_bundle": {
                "id": "bundle.example",
                "version": "1.0.0",
                "publisher": {"id":"acme", "name":"Acme", "publicKey":""},
                "requestedGrants": []
            }
        });
        let entries = vec![
            ArchiveEntry {
                name: "manifest.json".into(),
                bytes: serde_json::to_vec(&manifest).unwrap(),
            },
            ArchiveEntry {
                name: "index.js".into(),
                bytes: b"export default 1;".to_vec(),
            },
        ];
        (manifest, entries)
    }

    fn signed_review_fixture() -> (Value, Vec<ArchiveEntry>, VerifyingKey) {
        let signing_key = SigningKey::generate(&mut OsRng);
        let (mut manifest, mut entries) = unsigned_entries();
        let digest = canonical_payload_digest(&entries).unwrap();
        let digest_hex = hex::encode(digest);
        let signature = signing_key.sign(&signature_message(MOD_REVIEW_DOMAIN, &digest));
        let bundle = manifest["capability_bundle"].as_object_mut().unwrap();
        bundle.insert("payloadSha256".into(), json!(digest_hex));
        bundle.insert(
            "reviewSignature".into(),
            json!({
                "algorithm":"ed25519",
                "keyId": REVIEW_KEY_ID,
                "payloadSha256": digest_hex,
                "signature": hex::encode(signature.to_bytes())
            }),
        );
        entries[0].bytes = serde_json::to_vec(&manifest).unwrap();
        (manifest, entries, signing_key.verifying_key())
    }

    #[test]
    fn valid_review_signature_is_reviewed() {
        let (manifest, entries, key) = signed_review_fixture();
        let trust = evaluate_package_with_key(&manifest, &entries, &key).unwrap();
        assert_eq!(trust.review_state, "reviewed");
        assert_eq!(trust.integrity_state, "verified");
    }

    #[test]
    fn pinned_review_key_and_wire_protocol_are_stable() {
        assert_eq!(
            hex::encode(ELDRIS_REVIEW_PUBLIC_KEY),
            "d40713a67f6ec73f2cadfa89bbc92d4535055655d368cc0606051b6b60f29620"
        );
        assert_eq!(
            production_review_key().unwrap().to_bytes(),
            ELDRIS_REVIEW_PUBLIC_KEY
        );
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let digest: [u8; 32] = Sha256::digest(b"oomu-mod-review-kat-v1").into();
        assert_eq!(
            hex::encode(digest),
            "3102f56ac0847ab68fa558bd1b4697cc6682737fc7fef43409b3ff6c29217200"
        );
        let signature = signing_key.sign(&signature_message(MOD_REVIEW_DOMAIN, &digest));
        assert_eq!(
            hex::encode(signature.to_bytes()),
            "fd3979ab5893db116e30ed8a9bbaf79d55bff0c75030d37375d4862a1e37bf09f7a761c59420716605078070d255502fefba89085adbc84bf694378a7cc0610b"
        );
    }

    #[test]
    fn detached_review_signature_can_verify_without_manifest_digest() {
        let (mut manifest, mut entries, key) = signed_review_fixture();
        let envelope = manifest["capability_bundle"]
            .as_object_mut()
            .unwrap()
            .remove("reviewSignature")
            .unwrap();
        manifest["capability_bundle"]
            .as_object_mut()
            .unwrap()
            .remove("payloadSha256");
        entries[0].bytes = serde_json::to_vec(&manifest).unwrap();
        entries.push(ArchiveEntry {
            name: DETACHED_REVIEW_SIGNATURE_PATH.into(),
            bytes: serde_json::to_vec(&envelope).unwrap(),
        });
        let trust = evaluate_package_with_key(&manifest, &entries, &key).unwrap();
        assert_eq!(trust.review_state, "reviewed");
        assert_eq!(trust.integrity_state, "verified");
    }

    #[test]
    fn unsigned_package_is_unreviewed() {
        let (manifest, entries) = unsigned_entries();
        let key = SigningKey::generate(&mut OsRng).verifying_key();
        let trust = evaluate_package_with_key(&manifest, &entries, &key).unwrap();
        assert_eq!(trust.review_state, "unreviewed");
        assert_eq!(trust.integrity_state, "unsigned");
    }

    #[test]
    fn valid_developer_signature_is_verified_but_unreviewed() {
        let publisher = SigningKey::generate(&mut OsRng);
        let review_key = SigningKey::generate(&mut OsRng).verifying_key();
        let (mut manifest, mut entries) = unsigned_entries();
        manifest["capability_bundle"]["publisher"]["publicKey"] =
            json!(hex::encode(publisher.verifying_key().to_bytes()));
        entries[0].bytes = serde_json::to_vec(&manifest).unwrap();
        let digest = canonical_payload_digest(&entries).unwrap();
        manifest["capability_bundle"]["payloadSha256"] = json!(hex::encode(digest));
        manifest["capability_bundle"]["signature"] = json!({
            "algorithm": "ed25519",
            "payloadSha256": hex::encode(digest),
            "signature": hex::encode(
                publisher
                    .sign(&signature_message(MOD_PUBLISHER_DOMAIN, &digest))
                    .to_bytes()
            )
        });
        entries[0].bytes = serde_json::to_vec(&manifest).unwrap();
        let trust = evaluate_package_with_key(&manifest, &entries, &review_key).unwrap();
        assert_eq!(trust.review_state, "unreviewed");
        assert!(trust.publisher_identity_verified);
        assert_eq!(trust.integrity_state, "verified");
    }

    #[test]
    fn tampered_payload_is_modified() {
        let (manifest, mut entries, key) = signed_review_fixture();
        entries[1].bytes.push(b'!');
        let trust = evaluate_package_with_key(&manifest, &entries, &key).unwrap();
        assert_eq!(trust.review_state, "unreviewed");
        assert_eq!(trust.integrity_state, "modified");
    }

    #[test]
    fn malformed_signature_is_modified_without_panicking() {
        let (mut manifest, mut entries, key) = signed_review_fixture();
        manifest["capability_bundle"]["reviewSignature"]["signature"] = json!("not-hex");
        entries[0].bytes = serde_json::to_vec(&manifest).unwrap();
        let trust = evaluate_package_with_key(&manifest, &entries, &key).unwrap();
        assert_eq!(trust.review_state, "unreviewed");
        assert_eq!(trust.integrity_state, "modified");
    }

    #[test]
    fn canonical_digest_rejects_duplicate_paths() {
        let (_, mut entries) = unsigned_entries();
        entries.push(ArchiveEntry {
            name: "INDEX.JS".into(),
            bytes: Vec::new(),
        });
        assert!(canonical_payload_digest(&entries).is_err());
    }

    #[test]
    fn old_installed_mod_schema_upgrades_in_place() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch("CREATE TABLE installed_mods (id TEXT PRIMARY KEY, name TEXT NOT NULL);")
            .unwrap();
        ensure_installed_mod_trust_columns(&connection).unwrap();
        for column in [
            "review_state",
            "publisher_identity_verified",
            "integrity_state",
            "payload_sha256",
            "is_built_in",
        ] {
            assert!(table_has_column(&connection, "installed_mods", column).unwrap());
        }
    }
}
