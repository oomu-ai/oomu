use crate::foundation::{clock::unix_time_ms_i64 as unix_time_ms, digest::sha256_hex};
use argon2::{Algorithm, Argon2, Params, Version};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use zeroize::{Zeroize, Zeroizing};

mod signature_domains;
pub(crate) use signature_domains::{
    NativeFileAuthorityClaim, NativeFileAuthorityEnvelope, NATIVE_FILE_AUTHORITY_VERSION,
};

const PERSISTENT_NODE_DERIVATION_DOMAIN: &[u8] = b"oomu-persistent-node-seed-v1";
const IDENTITY_PROFILE: &str = "sovereign_identity.json";
const OOMU_IDENTITY_DIR: &str = ".oomu/identity";
const NODE_IDENTITY_FILE: &str = "node_identity.json";
const LEGACY_NODE_PRIVATE_KEY_FILE: &str = "node_ed25519.key";
const IDENTITY_PROFILE_VERSION: u32 = 2;
const MAX_QUARANTINED_PREDECESSOR_KEYS: usize = 64;
const SESSION_PASSPHRASE_MIN_CHARS: usize = 14;
const SESSION_PASSPHRASE_MEMORY_KIB: u32 = 19 * 1024;
const SESSION_PASSPHRASE_ITERATIONS: u32 = 3;
const SESSION_PASSPHRASE_PARALLELISM: u32 = 1;

#[cfg(test)]
pub(crate) static APP_DATA_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Clone)]
pub struct SovereignIdentity {
    profile_path: Arc<PathBuf>,
    session_nonce: Arc<Mutex<String>>,
    tenant_context: Arc<Mutex<TenantContext>>,
    session_identity_material: Arc<Mutex<Option<SessionIdentityMaterial>>>,
    #[cfg(test)]
    signing_key_override: Arc<Option<SigningKey>>,
    #[cfg(test)]
    node_signing_key_override: Arc<Option<SigningKey>>,
}

#[derive(Clone)]
struct SessionIdentityMaterial {
    root_signing_key: SigningKey,
    node_signing_key: SigningKey,
    node_profile: NodeIdentityProfile,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct SignatureBlock {
    pub public_key: String,
    pub signature: String,
    pub payload_hash: String,
    pub signed_at_ms: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TenantRole {
    Commander,
    CommandStaff,
    Patrol,
    InternalAffairs,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TenantContext {
    pub tenant_id: String,
    pub tenant_label: String,
    pub role: TenantRole,
    pub classification: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TenantSubKey {
    pub key_id: String,
    pub tenant_id: String,
    pub role: TenantRole,
    pub classification: String,
    pub public_material_hash: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VerifyArtifactRequest {
    pub content: String,
    pub signature: SignatureBlock,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DelegateSigningAuthorityRequest {
    pub remote_node_id: String,
    pub mission_id: String,
    pub allowed_operations: Vec<String>,
    pub expires_in_ms: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct IdentityProfile {
    pub public_key: String,
    pub fingerprint: String,
    pub hardware_binding: String,
    pub storage_backend: String,
    pub genesis_created_at_ms: i64,
    pub session_nonce: String,
    pub tenant_context: TenantContext,
    pub tenant_subkeys: Vec<TenantSubKey>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct PersistedIdentityProfile {
    #[serde(default)]
    profile_version: u32,
    public_key: String,
    #[serde(default)]
    fingerprint: String,
    #[serde(default)]
    hardware_binding: String,
    #[serde(default)]
    storage_backend: String,
    #[serde(default)]
    genesis_created_at_ms: i64,
    #[serde(default)]
    quarantined_predecessor_public_keys: Vec<String>,
    #[serde(default)]
    key_rotations: Vec<IdentityKeyRotation>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct IdentityKeyRotation {
    previous_fingerprint: String,
    activated_fingerprint: String,
    rotated_at_ms: i64,
    reason: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NodeIdentityProfile {
    pub node_id: String,
    pub public_key: String,
    pub public_key_fingerprint: String,
    pub private_key_path: String,
    pub identity_dir: String,
    pub architect_signature: SignatureBlock,
    pub created_at_ms: i64,
}

#[derive(Debug, Serialize)]
pub struct VerificationResponse {
    pub verified: bool,
    pub message: String,
    pub public_key: String,
    pub payload_hash: String,
}

#[derive(Debug, Serialize)]
pub struct DelegatedSigningAuthority {
    pub remote_node_id: String,
    pub mission_id: String,
    pub allowed_operations: Vec<String>,
    pub expires_at_ms: i64,
    pub delegation_payload: String,
    pub principal_signature: SignatureBlock,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct IdentityRecoveryOption {
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ManualSessionPassphraseRequest {
    pub passphrase: String,
}

#[derive(Debug, Serialize)]
pub struct ManualSessionPassphraseResponse {
    pub public_key: String,
    pub fingerprint: String,
    pub storage_backend: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct IdentityError {
    pub code: &'static str,
    pub boundary: &'static str,
    pub message: String,
    pub recovery_options: Vec<IdentityRecoveryOption>,
}

pub type SovereignIdentityError = IdentityError;

impl SovereignIdentity {
    pub fn initialize() -> Result<Self, String> {
        let profile_path = project_root().join(IDENTITY_PROFILE);
        if let Some(parent) = profile_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let identity = Self::new(profile_path);
        identity.ensure_genesis().map_err(|error| error.message)?;
        identity
            .retire_legacy_local_node_key()
            .map_err(|error| error.message)?;
        Ok(identity)
    }

    pub fn initialize_interactive() -> Self {
        let profile_path = project_root().join(IDENTITY_PROFILE);
        if let Some(parent) = profile_path.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                eprintln!(
                    "SOVEREIGN_IDENTITY_PROFILE_DIR_UNAVAILABLE code=sovereign_identity_keyring_unavailable boundary=SovereignIdentity error={}",
                    error.kind()
                );
            }
        }
        let identity = Self::new(profile_path);
        // SQLCipher has already cached this root credential. Node receipts use
        // a domain-separated child key derived from the same cached root, so
        // startup and first execution never fan out into a second Keychain
        // password prompt.
        if let Err(error) = identity.ensure_genesis() {
            log_secure_identity_failure(&error);
        }
        identity
    }

    pub fn initialize_with_session_passphrase(passphrase: &str) -> Result<Self, IdentityError> {
        let profile_path = project_root().join(IDENTITY_PROFILE);
        if let Some(parent) = profile_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| IdentityError::secure_storage(error.to_string()))?;
        }
        let identity = Self::new_without_test_override(profile_path);
        identity.activate_manual_session_passphrase(passphrase)?;
        Ok(identity)
    }

    /// Retries secure storage only when the user explicitly asks. The shared
    /// session cache still prevents automatic Keychain prompt loops.
    pub fn retry_secure_storage_probe(&self) -> Result<IdentityProfile, IdentityError> {
        let mut seed_hex = retry_keychain_password()
            .map_err(IdentityError::keyring_unavailable)?
            .ok_or_else(|| {
                IdentityError::keyring_unavailable(
                    "Sovereign Identity signing key is unavailable.".to_string(),
                )
            })?;
        let validation = signing_key_from_seed_hex(&seed_hex);
        seed_hex.zeroize();
        let signing_key = validation?;
        if self.profile_path.exists() {
            self.reconcile_profile_with_signing_key(&signing_key)?;
        } else {
            self.persist_genesis_profile(&signing_key)?;
        }
        self.retire_legacy_local_node_key()?;
        self.profile()
    }

    #[cfg(test)]
    pub(crate) fn initialize_ephemeral() -> Self {
        Self {
            profile_path: Arc::new(project_root().join("test_sovereign_identity.json")),
            session_nonce: Arc::new(Mutex::new(new_session_nonce())),
            tenant_context: Arc::new(Mutex::new(TenantContext::greece_police_commander())),
            session_identity_material: Arc::new(Mutex::new(None)),
            signing_key_override: Arc::new(Some(SigningKey::generate(&mut OsRng))),
            node_signing_key_override: Arc::new(Some(SigningKey::generate(&mut OsRng))),
        }
    }

    fn new(profile_path: PathBuf) -> Self {
        Self {
            profile_path: Arc::new(profile_path),
            session_nonce: Arc::new(Mutex::new(new_session_nonce())),
            tenant_context: Arc::new(Mutex::new(TenantContext::greece_police_commander())),
            session_identity_material: Arc::new(Mutex::new(None)),
            #[cfg(test)]
            signing_key_override: Arc::new(Some(test_signing_key())),
            #[cfg(test)]
            node_signing_key_override: Arc::new(Some(test_node_signing_key())),
        }
    }

    fn new_without_test_override(profile_path: PathBuf) -> Self {
        Self {
            profile_path: Arc::new(profile_path),
            session_nonce: Arc::new(Mutex::new(new_session_nonce())),
            tenant_context: Arc::new(Mutex::new(TenantContext::greece_police_commander())),
            session_identity_material: Arc::new(Mutex::new(None)),
            #[cfg(test)]
            signing_key_override: Arc::new(None),
            #[cfg(test)]
            node_signing_key_override: Arc::new(Some(test_node_signing_key())),
        }
    }

    pub fn profile(&self) -> Result<IdentityProfile, IdentityError> {
        self.ensure_genesis()?;
        let public_key = self.public_key_without_keychain()?;
        let profile = IdentityProfile {
            fingerprint: fingerprint(&public_key),
            public_key,
            hardware_binding: hardware_binding(),
            storage_backend: self.storage_backend(),
            genesis_created_at_ms: read_genesis_created_at(&self.profile_path),
            session_nonce: self
                .session_nonce
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone(),
            tenant_context: self.tenant_context(),
            tenant_subkeys: self.tenant_subkeys()?,
        };
        Ok(profile)
    }

    pub fn tenant_context(&self) -> TenantContext {
        self.tenant_context
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn tenant_subkeys(&self) -> Result<Vec<TenantSubKey>, IdentityError> {
        let context = self.tenant_context();
        Ok(vec![
            self.derive_tenant_subkey(&context, "mission"),
            self.derive_tenant_subkey(&context, "retrieval"),
            self.derive_tenant_subkey(&context, "audit"),
        ])
    }

    pub fn silo_key_hash(
        &self,
        context: &TenantContext,
        purpose: &str,
    ) -> Result<String, IdentityError> {
        Ok(self
            .derive_tenant_subkey(context, purpose)
            .public_material_hash)
    }

    pub fn sign_payload(&self, payload: &str) -> Result<SignatureBlock, IdentityError> {
        let signing_key = self.load_signing_key()?;
        let domain_payload = signature_domains::native_evidence_payload(payload);
        let signature = signing_key.sign(domain_payload.as_bytes());
        Ok(SignatureBlock {
            public_key: hex::encode(signing_key.verifying_key().to_bytes()),
            signature: hex::encode(signature.to_bytes()),
            payload_hash: sha256_hex(payload.as_bytes()),
            signed_at_ms: unix_time_ms(),
        })
    }

    pub(crate) fn sign_exact_payload(
        &self,
        payload: &str,
    ) -> Result<SignatureBlock, IdentityError> {
        let signing_key = self.load_signing_key()?;
        let signature = signing_key.sign(payload.as_bytes());
        Ok(SignatureBlock {
            public_key: hex::encode(signing_key.verifying_key().to_bytes()),
            signature: hex::encode(signature.to_bytes()),
            payload_hash: sha256_hex(payload.as_bytes()),
            signed_at_ms: unix_time_ms(),
        })
    }

    pub fn verify_payload(
        &self,
        payload: &str,
        signature_block: &SignatureBlock,
    ) -> Result<(), IdentityError> {
        if !self.signature_uses_current_key(signature_block)? {
            return Err(IdentityError::integrity(
                "Ledger Integrity Violation: signature public key does not match local Sovereign Identity.",
            ));
        }
        if signature_block.payload_hash != sha256_hex(payload.as_bytes()) {
            return Err(IdentityError::integrity(
                "Ledger Integrity Violation: payload hash does not match signature block.",
            ));
        }
        let domain_payload = signature_domains::native_evidence_payload(payload);
        if verify_signature_bytes_with_public_key(&domain_payload, signature_block).is_ok() {
            return Ok(());
        }
        // Existing envelopes remain readable, but no authority validator calls
        // this compatibility path for a new action.
        verify_signature_bytes_with_public_key(payload, signature_block)
    }

    pub(crate) fn verify_exact_current_payload(
        &self,
        payload: &str,
        signature_block: &SignatureBlock,
    ) -> Result<(), IdentityError> {
        if !self.signature_uses_current_key(signature_block)? {
            return Err(IdentityError::integrity(
                "Ledger Integrity Violation: signature public key does not match local Sovereign Identity.",
            ));
        }
        verify_payload_with_public_key(payload, signature_block)
    }

    pub fn verify_architect_signature(
        &self,
        payload: &str,
        signature_block: &SignatureBlock,
    ) -> Result<(), IdentityError> {
        self.verify_payload(payload, signature_block).map_err(|_| {
            IdentityError::integrity("Architect Root Key mismatch for mesh identity proof.")
        })
    }

    pub fn node_identity(&self) -> Result<NodeIdentityProfile, IdentityError> {
        if let Some(identity) = self
            .session_identity_material
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
        {
            return Ok(identity.node_profile.clone());
        }
        let path = node_identity_path()?;
        let content = fs::read_to_string(&path).map_err(|error| {
            IdentityError::secure_storage(format!("Node identity is unavailable: {error}"))
        })?;
        serde_json::from_str(&content).map_err(|error| IdentityError::invalid(error.to_string()))
    }

    pub fn generate_node_identity(&self) -> Result<NodeIdentityProfile, IdentityError> {
        if let Some(identity) = self
            .session_identity_material
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
        {
            return Ok(identity.node_profile.clone());
        }
        self.retire_legacy_local_node_key()?;

        let identity_dir = oomu_identity_dir()?;
        fs::create_dir_all(&identity_dir)
            .map_err(|error| IdentityError::secure_storage(error.to_string()))?;

        let identity_path = identity_dir.join(NODE_IDENTITY_FILE);
        let signing_key = self.load_node_signing_key()?;
        let public_key = hex::encode(signing_key.verifying_key().to_bytes());
        if identity_path.exists() {
            let existing = self.node_identity()?;
            if existing.public_key == public_key
                && existing.private_key_path == node_private_key_location()
            {
                return Ok(existing);
            }
            eprintln!(
                "NODE_IDENTITY_ROTATED reason=consolidated_root_derived_key previous_fingerprint={} active_fingerprint={}",
                existing.public_key_fingerprint,
                fingerprint(&public_key)
            );
        }
        let node_id = format!(
            "oomu-node-{}",
            fingerprint(&public_key)
                .chars()
                .take(12)
                .collect::<String>()
        );
        let created_at_ms = unix_time_ms();
        let proof_payload = node_identity_payload(&node_id, &public_key, created_at_ms);
        let architect_signature = self.sign_payload(&proof_payload)?;
        let profile = NodeIdentityProfile {
            node_id,
            public_key: public_key.clone(),
            public_key_fingerprint: fingerprint(&public_key),
            private_key_path: node_private_key_location(),
            identity_dir: identity_dir.to_string_lossy().to_string(),
            architect_signature,
            created_at_ms,
        };
        fs::write(
            &identity_path,
            serde_json::to_string_pretty(&profile)
                .map_err(|error| IdentityError::invalid(error.to_string()))?,
        )
        .map_err(|error| IdentityError::secure_storage(error.to_string()))?;
        Ok(profile)
    }

    pub fn sign_node_payload(&self, payload: &str) -> Result<SignatureBlock, IdentityError> {
        if let Some(identity) = self
            .session_identity_material
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
        {
            return Ok(signature_block_from_signing_key(
                &identity.node_signing_key,
                payload,
            ));
        }

        #[cfg(test)]
        if let Some(signing_key) = self.node_signing_key_override.as_ref() {
            return Ok(signature_block_from_signing_key(signing_key, payload));
        }

        let _profile = self.generate_node_identity()?;
        let signing_key = self.load_node_signing_key()?;
        Ok(signature_block_from_signing_key(&signing_key, payload))
    }

    pub(crate) fn sign_node_payload_with_profile(
        &self,
        payload: &str,
    ) -> Result<(NodeIdentityProfile, SignatureBlock), IdentityError> {
        if let Some(material) = self
            .session_identity_material
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
        {
            return Ok((
                material.node_profile.clone(),
                signature_block_from_signing_key(&material.node_signing_key, payload),
            ));
        }

        let profile = self.generate_node_identity()?;
        let signature = self.sign_node_payload(payload)?;
        if signature.public_key != profile.public_key {
            return Err(IdentityError::integrity(
                "Node identity changed while the operation receipt was being signed.",
            ));
        }
        Ok((profile, signature))
    }

    fn retire_legacy_local_node_key(&self) -> Result<(), IdentityError> {
        let identity_dir = oomu_identity_dir()?;
        let legacy_key_file = identity_dir.join(LEGACY_NODE_PRIVATE_KEY_FILE);
        if !legacy_key_file.exists() {
            return Ok(());
        }

        securely_remove_legacy_node_key(&legacy_key_file)?;
        eprintln!("NODE_IDENTITY_LEGACY_KEY_RETIRED storage=root_derived legacy_file=erased");
        Ok(())
    }

    fn load_node_signing_key(&self) -> Result<SigningKey, IdentityError> {
        #[cfg(test)]
        if let Some(signing_key) = self.node_signing_key_override.as_ref() {
            return Ok(signing_key.clone());
        }

        let mut root_seed_hex = get_keychain_password()
            .map_err(IdentityError::keyring_unavailable)?
            .ok_or_else(|| {
                IdentityError::keyring_unavailable(
                    "Sovereign Identity signing key is unavailable.".to_string(),
                )
            })?;
        let derived = derive_persistent_node_seed(&root_seed_hex);
        root_seed_hex.zeroize();
        let seed = Zeroizing::new(derived?);
        Ok(SigningKey::from_bytes(&seed))
    }

    pub fn verify_node_payload(
        &self,
        payload: &str,
        signature_block: &SignatureBlock,
    ) -> Result<(), IdentityError> {
        verify_payload_with_public_key(payload, signature_block)
    }

    pub fn sign_release_artifact_payload(
        &self,
        payload: &str,
    ) -> Result<SignatureBlock, IdentityError> {
        self.sign_payload(payload)
    }

    pub fn mesh_handshake_log_path() -> Result<PathBuf, IdentityError> {
        Ok(oomu_identity_dir()?.join("mesh_handshake.log"))
    }
}

pub fn node_identity_payload(node_id: &str, public_key: &str, created_at_ms: i64) -> String {
    serde_json::json!({
        "node_id": node_id,
        "public_key": public_key,
        "created_at_ms": created_at_ms,
        "proof": "oomu_node_identity"
    })
    .to_string()
}

fn verify_payload_with_public_key(
    payload: &str,
    signature_block: &SignatureBlock,
) -> Result<(), IdentityError> {
    let payload_hash = sha256_hex(payload.as_bytes());
    if signature_block.payload_hash != payload_hash {
        return Err(IdentityError::integrity(
            "Ledger Integrity Violation: payload hash does not match signature block.",
        ));
    }

    verify_signature_bytes_with_public_key(payload, signature_block)
}

fn verify_signature_bytes_with_public_key(
    payload: &str,
    signature_block: &SignatureBlock,
) -> Result<(), IdentityError> {
    let public_key_bytes = hex::decode(&signature_block.public_key)
        .map_err(|error| IdentityError::invalid(error.to_string()))?;
    let public_key_array: [u8; 32] = public_key_bytes
        .try_into()
        .map_err(|_| IdentityError::invalid("Invalid Ed25519 public key length.".to_string()))?;
    let verifying_key = VerifyingKey::from_bytes(&public_key_array)
        .map_err(|error| IdentityError::invalid(error.to_string()))?;
    let signature_bytes = hex::decode(&signature_block.signature)
        .map_err(|error| IdentityError::invalid(error.to_string()))?;
    let signature = Signature::from_slice(&signature_bytes)
        .map_err(|error| IdentityError::invalid(error.to_string()))?;
    verifying_key
        .verify(payload.as_bytes(), &signature)
        .map_err(|_| {
            IdentityError::integrity("Ledger Integrity Violation: signature verification failed.")
        })
}

fn signature_block_from_signing_key(signing_key: &SigningKey, payload: &str) -> SignatureBlock {
    let signature = signing_key.sign(payload.as_bytes());
    SignatureBlock {
        public_key: hex::encode(signing_key.verifying_key().to_bytes()),
        signature: hex::encode(signature.to_bytes()),
        payload_hash: sha256_hex(payload.as_bytes()),
        signed_at_ms: unix_time_ms(),
    }
}

impl SovereignIdentity {
    pub fn activate_manual_session_passphrase(
        &self,
        passphrase: &str,
    ) -> Result<ManualSessionPassphraseResponse, IdentityError> {
        validate_session_passphrase(passphrase)?;
        let passphrase_bytes = Zeroizing::new(passphrase.as_bytes().to_vec());
        let seed = Zeroizing::new(derive_session_seed(&passphrase_bytes)?);
        let node_seed = Zeroizing::new(derive_session_node_seed(&passphrase_bytes)?);
        let signing_key = SigningKey::from_bytes(&seed);
        let node_signing_key = SigningKey::from_bytes(&node_seed);
        let public_key = hex::encode(signing_key.verifying_key().to_bytes());
        let node_profile = Self::manual_session_node_profile(&signing_key, &node_signing_key)?;
        let mut session_material = self
            .session_identity_material
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *session_material = Some(SessionIdentityMaterial {
            root_signing_key: signing_key,
            node_signing_key,
            node_profile,
        });
        Ok(ManualSessionPassphraseResponse {
            fingerprint: fingerprint(&public_key),
            public_key,
            storage_backend: "manual_session_passphrase_memory_only".to_string(),
            message: "Manual session passphrase accepted. Sovereign Identity signing is active for this process only; no private key material was written to disk.".to_string(),
        })
    }

    pub fn clear_sensitive_session_material(&self) {
        let mut session_material = self
            .session_identity_material
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *session_material = None;
    }

    fn manual_session_node_profile(
        root_signing_key: &SigningKey,
        node_signing_key: &SigningKey,
    ) -> Result<NodeIdentityProfile, IdentityError> {
        let public_key = hex::encode(node_signing_key.verifying_key().to_bytes());
        let node_id = format!(
            "oomu-node-{}",
            fingerprint(&public_key)
                .chars()
                .take(12)
                .collect::<String>()
        );
        let created_at_ms = unix_time_ms();
        let architect_signature = signature_block_from_signing_key(
            root_signing_key,
            &node_identity_payload(&node_id, &public_key, created_at_ms),
        );
        Ok(NodeIdentityProfile {
            node_id,
            public_key: public_key.clone(),
            public_key_fingerprint: fingerprint(&public_key),
            private_key_path: "memory:manual-session-passphrase-node-key".to_string(),
            identity_dir: oomu_identity_dir()?.to_string_lossy().to_string(),
            architect_signature,
            created_at_ms,
        })
    }

    fn storage_backend(&self) -> String {
        let session_material = self
            .session_identity_material
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if session_material.is_some() {
            "manual session passphrase (memory only)".to_string()
        } else {
            "OS keychain".to_string()
        }
    }

    fn ensure_genesis(&self) -> Result<(), IdentityError> {
        #[cfg(test)]
        if self.signing_key_override.is_some() {
            return Ok(());
        }

        if self
            .session_identity_material
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
        {
            return Ok(());
        }

        match get_keychain_password().map_err(IdentityError::keyring_unavailable)? {
            Some(mut seed_hex) => {
                let signing_key = signing_key_from_seed_hex(&seed_hex);
                seed_hex.zeroize();
                let signing_key = signing_key?;
                if self.profile_path.exists() {
                    self.reconcile_profile_with_signing_key(&signing_key)?;
                } else {
                    self.persist_genesis_profile(&signing_key)?;
                }
                return Ok(());
            }
            None => {}
        }

        let signing_key = SigningKey::generate(&mut OsRng);
        let mut seed_hex = hex::encode(signing_key.to_bytes());

        if let Err(error) =
            set_keychain_password(&seed_hex).map_err(IdentityError::keyring_unavailable)
        {
            seed_hex.zeroize();
            log_secure_identity_failure(&error);
            return Err(IdentityError::keyring_unavailable(format!(
                "Keyring write failed. Private key material was not persisted to any local plaintext fallback. {}",
                error.message
            )));
        }
        seed_hex.zeroize();

        if self.profile_path.exists() {
            self.reconcile_profile_with_signing_key(&signing_key)
        } else {
            self.persist_genesis_profile(&signing_key)
        }
    }

    fn persist_genesis_profile(&self, signing_key: &SigningKey) -> Result<(), IdentityError> {
        let public_key = hex::encode(signing_key.verifying_key().to_bytes());
        let profile = PersistedIdentityProfile {
            profile_version: IDENTITY_PROFILE_VERSION,
            fingerprint: fingerprint(&public_key),
            public_key,
            hardware_binding: hardware_binding(),
            storage_backend: "OS keychain".to_string(),
            genesis_created_at_ms: unix_time_ms(),
            quarantined_predecessor_public_keys: Vec::new(),
            key_rotations: Vec::new(),
        };
        self.write_persisted_profile(&profile)
    }

    fn reconcile_profile_with_signing_key(
        &self,
        signing_key: &SigningKey,
    ) -> Result<(), IdentityError> {
        let original_profile = fs::read(&*self.profile_path).map_err(|error| {
            IdentityError::secure_storage(format!(
                "Sovereign Identity public profile is unavailable: {error}"
            ))
        })?;
        let mut profile = serde_json::from_slice::<PersistedIdentityProfile>(&original_profile)
            .map_err(|error| IdentityError::invalid(error.to_string()))?;
        let persisted_public_key = normalize_public_key(&profile.public_key)?;
        let active_public_key = hex::encode(signing_key.verifying_key().to_bytes());

        let mut quarantined_predecessors = Vec::new();
        for public_key in &profile.quarantined_predecessor_public_keys {
            let public_key = normalize_public_key(public_key)?;
            if public_key != active_public_key && !quarantined_predecessors.contains(&public_key) {
                quarantined_predecessors.push(public_key);
            }
        }

        if persisted_public_key == active_public_key {
            return Ok(());
        }

        if !quarantined_predecessors.contains(&persisted_public_key) {
            quarantined_predecessors.push(persisted_public_key.clone());
        }
        if quarantined_predecessors.len() > MAX_QUARANTINED_PREDECESSOR_KEYS {
            return Err(IdentityError::integrity(
                "Sovereign Identity key history exceeds the supported safety limit.",
            ));
        }

        profile.key_rotations.push(IdentityKeyRotation {
            previous_fingerprint: fingerprint(&persisted_public_key),
            activated_fingerprint: fingerprint(&active_public_key),
            rotated_at_ms: unix_time_ms(),
            reason: "secure_storage_profile_reconciliation_quarantined".to_string(),
        });
        profile.profile_version = IDENTITY_PROFILE_VERSION;
        profile.public_key = active_public_key.clone();
        profile.fingerprint = fingerprint(&active_public_key);
        profile.hardware_binding = hardware_binding();
        profile.storage_backend = "OS keychain".to_string();
        if profile.genesis_created_at_ms <= 0 {
            profile.genesis_created_at_ms = unix_time_ms();
        }
        profile.quarantined_predecessor_public_keys = quarantined_predecessors;
        self.archive_profile_for_recovery(&original_profile)?;
        self.write_persisted_profile(&profile)?;

        eprintln!(
            "SOVEREIGN_IDENTITY_PROFILE_RECONCILED previous_fingerprint={} active_fingerprint={} quarantined_predecessors={}",
            fingerprint(&persisted_public_key),
            fingerprint(&active_public_key),
            profile.quarantined_predecessor_public_keys.len()
        );
        Ok(())
    }

    fn read_persisted_profile(&self) -> Result<PersistedIdentityProfile, IdentityError> {
        let content = fs::read_to_string(&*self.profile_path).map_err(|error| {
            IdentityError::secure_storage(format!(
                "Sovereign Identity public profile is unavailable: {error}"
            ))
        })?;
        serde_json::from_str(&content).map_err(|error| IdentityError::invalid(error.to_string()))
    }

    fn write_persisted_profile(
        &self,
        profile: &PersistedIdentityProfile,
    ) -> Result<(), IdentityError> {
        let content = serde_json::to_vec_pretty(profile)
            .map_err(|error| IdentityError::invalid(error.to_string()))?;
        let temporary_path = self.profile_path.with_extension(format!(
            "json.{}.{}.tmp",
            std::process::id(),
            unix_time_ms()
        ));
        let write_result = (|| -> Result<(), IdentityError> {
            let mut file = fs::File::create(&temporary_path)
                .map_err(|error| IdentityError::secure_storage(error.to_string()))?;
            file.write_all(&content)
                .map_err(|error| IdentityError::secure_storage(error.to_string()))?;
            file.sync_all()
                .map_err(|error| IdentityError::secure_storage(error.to_string()))?;
            fs::rename(&temporary_path, &*self.profile_path)
                .map_err(|error| IdentityError::secure_storage(error.to_string()))?;
            Ok(())
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&temporary_path);
        }
        write_result
    }

    fn archive_profile_for_recovery(&self, content: &[u8]) -> Result<(), IdentityError> {
        let recovery_path = self.profile_path.with_file_name(format!(
            "sovereign_identity.recovery.{}.json",
            sha256_hex(content)
        ));
        if recovery_path.exists() {
            return Ok(());
        }
        fs::write(recovery_path, content)
            .map_err(|error| IdentityError::secure_storage(error.to_string()))
    }

    fn public_key_without_keychain(&self) -> Result<String, IdentityError> {
        #[cfg(test)]
        if let Some(signing_key) = self.signing_key_override.as_ref() {
            return Ok(hex::encode(signing_key.verifying_key().to_bytes()));
        }

        if let Some(material) = self
            .session_identity_material
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .cloned()
        {
            return Ok(hex::encode(
                material.root_signing_key.verifying_key().to_bytes(),
            ));
        }

        normalize_public_key(&self.read_persisted_profile()?.public_key)
    }

    pub(crate) fn signature_uses_current_key(
        &self,
        signature_block: &SignatureBlock,
    ) -> Result<bool, IdentityError> {
        Ok(normalize_public_key(&signature_block.public_key)?
            == self.public_key_without_keychain()?)
    }

    fn load_signing_key(&self) -> Result<SigningKey, IdentityError> {
        #[cfg(test)]
        if let Some(signing_key) = self.signing_key_override.as_ref() {
            return Ok(signing_key.clone());
        }

        if let Some(material) = self
            .session_identity_material
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .cloned()
        {
            return Ok(material.root_signing_key);
        }

        let mut seed_hex = get_keychain_password()
            .map_err(IdentityError::keyring_unavailable)?
            .ok_or_else(|| {
                IdentityError::keyring_unavailable(
                    "Sovereign Identity signing key is unavailable.".to_string(),
                )
            })?;

        let signing_key = signing_key_from_seed_hex(&seed_hex);
        seed_hex.zeroize();
        signing_key
    }

    fn derive_tenant_subkey(&self, context: &TenantContext, purpose: &str) -> TenantSubKey {
        let role = tenant_role_to_str(&context.role);
        let material = format!(
            "{}:{}:{}:{}:{}",
            context.tenant_id,
            context.classification,
            role,
            purpose,
            hardware_binding()
        );
        TenantSubKey {
            key_id: format!("{}:{}:{purpose}", context.tenant_id, role),
            tenant_id: context.tenant_id.clone(),
            role: context.role.clone(),
            classification: context.classification.clone(),
            public_material_hash: sha256_hex(material.as_bytes()),
        }
    }

    pub fn delegate_signing_authority(
        &self,
        request: DelegateSigningAuthorityRequest,
    ) -> Result<DelegatedSigningAuthority, IdentityError> {
        let expires_at_ms = unix_time_ms() + request.expires_in_ms.unwrap_or(3_600_000).max(60_000);
        let payload = serde_json::json!({
            "remote_node_id": request.remote_node_id,
            "mission_id": request.mission_id,
            "allowed_operations": request.allowed_operations,
            "expires_at_ms": expires_at_ms,
            "delegation_kind": "ephemeral_research_signing"
        })
        .to_string();
        let signature = self.sign_payload(&payload)?;
        Ok(DelegatedSigningAuthority {
            remote_node_id: request.remote_node_id,
            mission_id: request.mission_id,
            allowed_operations: request.allowed_operations,
            expires_at_ms,
            delegation_payload: payload,
            principal_signature: signature,
            message: "Ephemeral delegation issued; final mission seal remains bound to primary Sovereign Identity.".to_string(),
        })
    }
}

impl TenantContext {
    pub fn greece_police_commander() -> Self {
        Self {
            tenant_id: "greece-police".to_string(),
            tenant_label: "Greece Police Department Pilot".to_string(),
            role: TenantRole::Commander,
            classification: "command".to_string(),
        }
    }
}

#[tauri::command]
pub async fn get_sovereign_identity(
    identity: tauri::State<'_, SovereignIdentity>,
) -> Result<IdentityProfile, IdentityError> {
    identity.profile()
}

#[tauri::command]
pub async fn generate_node_identity(
    identity: tauri::State<'_, SovereignIdentity>,
) -> Result<NodeIdentityProfile, IdentityError> {
    identity.generate_node_identity()
}

#[tauri::command]
pub async fn verify_artifact_signature(
    request: VerifyArtifactRequest,
    identity: tauri::State<'_, SovereignIdentity>,
) -> Result<VerificationResponse, IdentityError> {
    identity.verify_payload(&request.content, &request.signature)?;
    Ok(VerificationResponse {
        verified: true,
        message: "Artifact signature matches local Sovereign Identity.".to_string(),
        public_key: request.signature.public_key,
        payload_hash: request.signature.payload_hash,
    })
}

#[tauri::command]
pub async fn delegate_signing_authority(
    request: DelegateSigningAuthorityRequest,
    identity: tauri::State<'_, SovereignIdentity>,
) -> Result<DelegatedSigningAuthority, IdentityError> {
    identity.delegate_signing_authority(request)
}

#[tauri::command]
pub async fn activate_sovereign_identity_session_passphrase(
    request: ManualSessionPassphraseRequest,
    identity: tauri::State<'_, SovereignIdentity>,
) -> Result<ManualSessionPassphraseResponse, IdentityError> {
    identity.activate_manual_session_passphrase(&request.passphrase)
}

fn node_private_key_location() -> String {
    let (service, account) = keychain_location();
    format!("derived:{service}:{account}:node-v1")
}

fn keychain_location() -> (&'static str, &'static str) {
    crate::keychain_namespace::sovereign_identity_location()
}

fn get_keychain_password() -> Result<Option<String>, String> {
    let (service, account) = keychain_location();
    crate::keychain_session::get_password(service, account)
}

fn retry_keychain_password() -> Result<Option<String>, String> {
    let (service, account) = keychain_location();
    crate::keychain_session::retry_password(service, account)
}

fn set_keychain_password(secret: &str) -> Result<(), String> {
    let (service, account) = keychain_location();
    crate::keychain_session::set_password(service, account, secret)
}

fn derive_persistent_node_seed(root_seed_hex: &str) -> Result<[u8; 32], IdentityError> {
    let mut root_seed = hex::decode(root_seed_hex.trim())
        .map_err(|error| IdentityError::invalid(error.to_string()))?;
    if root_seed.len() != 32 {
        root_seed.zeroize();
        return Err(IdentityError::invalid(
            "Invalid Sovereign Identity seed length.".to_string(),
        ));
    }
    let mut digest = Sha256::new();
    digest.update(PERSISTENT_NODE_DERIVATION_DOMAIN);
    digest.update([0]);
    digest.update(hardware_binding().as_bytes());
    digest.update([0]);
    digest.update(&root_seed);
    root_seed.zeroize();
    let derived: [u8; 32] = digest.finalize().into();
    Ok(derived)
}

fn securely_remove_legacy_node_key(path: &Path) -> Result<(), IdentityError> {
    let file_len = fs::metadata(path)
        .map_err(|error| {
            IdentityError::secure_storage(format!("Secure wipe stat failed: {error}"))
        })?
        .len()
        .max(1);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|error| {
            IdentityError::secure_storage(format!("Secure wipe open failed: {error}"))
        })?;
    let zeros = [0_u8; 4096];
    let mut remaining = file_len;
    while remaining > 0 {
        let write_len = remaining.min(zeros.len() as u64) as usize;
        file.write_all(&zeros[..write_len]).map_err(|error| {
            IdentityError::secure_storage(format!("Secure wipe write failed: {error}"))
        })?;
        remaining -= write_len as u64;
    }
    file.sync_all().map_err(|error| {
        IdentityError::secure_storage(format!("Secure wipe sync failed: {error}"))
    })?;
    drop(file);
    fs::remove_file(path).map_err(|error| {
        IdentityError::secure_storage(format!("Failed to delete legacy node key: {error}"))
    })
}

fn signing_key_from_seed_hex(seed_hex: &str) -> Result<SigningKey, IdentityError> {
    let mut seed =
        hex::decode(seed_hex).map_err(|error| IdentityError::invalid(error.to_string()))?;
    let seed_array: [u8; 32] = seed
        .as_slice()
        .try_into()
        .map_err(|_| IdentityError::invalid("Invalid Ed25519 seed length.".to_string()))?;
    seed.zeroize();
    Ok(SigningKey::from_bytes(&seed_array))
}

fn validate_session_passphrase(passphrase: &str) -> Result<(), IdentityError> {
    let trimmed = passphrase.trim();
    if trimmed.chars().count() < SESSION_PASSPHRASE_MIN_CHARS {
        return Err(IdentityError::invalid(format!(
            "Manual session passphrase must be at least {SESSION_PASSPHRASE_MIN_CHARS} characters."
        )));
    }
    let has_letter = trimmed.chars().any(char::is_alphabetic);
    let has_non_letter = trimmed.chars().any(|character| !character.is_alphabetic());
    if !has_letter || !has_non_letter {
        return Err(IdentityError::invalid(
            "Manual session passphrase must mix letters with numbers, symbols, or spaces."
                .to_string(),
        ));
    }
    Ok(())
}

fn derive_session_seed(passphrase: &[u8]) -> Result<[u8; 32], IdentityError> {
    derive_session_seed_with_salt(passphrase, "oomu-sovereign-identity-session-v1")
}

fn derive_session_node_seed(passphrase: &[u8]) -> Result<[u8; 32], IdentityError> {
    derive_session_seed_with_salt(passphrase, "oomu-sovereign-identity-session-node-v1")
}

fn derive_session_seed_with_salt(
    passphrase: &[u8],
    salt_namespace: &str,
) -> Result<[u8; 32], IdentityError> {
    let mut seed = [0_u8; 32];
    let salt = format!(
        "{salt_namespace}:{}:{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    let params = Params::new(
        SESSION_PASSPHRASE_MEMORY_KIB,
        SESSION_PASSPHRASE_ITERATIONS,
        SESSION_PASSPHRASE_PARALLELISM,
        Some(seed.len()),
    )
    .map_err(|error| IdentityError::invalid(format!("Invalid Argon2id parameters: {error}")))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    if let Err(error) = argon2.hash_password_into(passphrase, salt.as_bytes(), &mut seed) {
        seed.zeroize();
        return Err(IdentityError::secure_storage(format!(
            "Argon2id derivation failed: {error}"
        )));
    }
    Ok(seed)
}

fn log_secure_identity_failure(error: &IdentityError) {
    eprintln!(
        "SOVEREIGN_IDENTITY_SECURE_FAILURE code={} boundary={}",
        error.code, error.boundary
    );
}

fn fingerprint(public_key: &str) -> String {
    sha256_hex(public_key.as_bytes())
        .chars()
        .take(32)
        .collect::<String>()
}

fn normalize_public_key(public_key: &str) -> Result<String, IdentityError> {
    let public_key_bytes = hex::decode(public_key.trim())
        .map_err(|error| IdentityError::invalid(error.to_string()))?;
    let public_key_array: [u8; 32] = public_key_bytes
        .try_into()
        .map_err(|_| IdentityError::invalid("Invalid Ed25519 public key length.".to_string()))?;
    VerifyingKey::from_bytes(&public_key_array)
        .map_err(|error| IdentityError::invalid(error.to_string()))?;
    Ok(hex::encode(public_key_array))
}

pub fn public_key_fingerprint(public_key: &str) -> String {
    fingerprint(public_key)
}

fn hardware_binding() -> String {
    sha256_hex(format!("{}:{}", std::env::consts::OS, std::env::consts::ARCH).as_bytes())
}

fn tenant_role_to_str(role: &TenantRole) -> &'static str {
    match role {
        TenantRole::Commander => "commander",
        TenantRole::CommandStaff => "command_staff",
        TenantRole::Patrol => "patrol",
        TenantRole::InternalAffairs => "internal_affairs",
    }
}

fn read_genesis_created_at(path: &Path) -> i64 {
    fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
        .and_then(|value| {
            value
                .get("genesis_created_at_ms")
                .and_then(|time| time.as_i64())
        })
        .unwrap_or_default()
}

fn new_session_nonce() -> String {
    sha256_hex(format!("{}:{}", unix_time_ms(), std::process::id()).as_bytes())
        .chars()
        .take(24)
        .collect()
}

#[cfg(test)]
fn test_signing_key() -> SigningKey {
    static TEST_SIGNING_KEY: std::sync::OnceLock<SigningKey> = std::sync::OnceLock::new();
    TEST_SIGNING_KEY
        .get_or_init(|| SigningKey::generate(&mut OsRng))
        .clone()
}

#[cfg(test)]
fn test_node_signing_key() -> SigningKey {
    static TEST_NODE_SIGNING_KEY: std::sync::OnceLock<SigningKey> = std::sync::OnceLock::new();
    TEST_NODE_SIGNING_KEY
        .get_or_init(|| SigningKey::generate(&mut OsRng))
        .clone()
}

fn project_root() -> PathBuf {
    crate::settings::app_data_root()
}

fn oomu_identity_dir() -> Result<PathBuf, IdentityError> {
    Ok(project_root().join(OOMU_IDENTITY_DIR))
}

fn node_identity_path() -> Result<PathBuf, IdentityError> {
    Ok(oomu_identity_dir()?.join(NODE_IDENTITY_FILE))
}

impl IdentityError {
    fn secure_storage(message: String) -> Self {
        Self {
            code: "identity_secure_storage_error",
            boundary: "SovereignIdentity",
            message,
            recovery_options: Vec::new(),
        }
    }

    fn keyring_unavailable(message: String) -> Self {
        Self {
            code: "sovereign_identity_keyring_unavailable",
            boundary: "SovereignIdentity",
            message,
            recovery_options: vec![
                IdentityRecoveryOption {
                    code: "manual_session_passphrase",
                    message: "Enter a manual session passphrase to derive a transient in-memory signing key. The key will not be written to disk."
                        .to_string(),
                },
                IdentityRecoveryOption {
                    code: "restore_host_keyring",
                    message: "Restore host keyring access through macOS Keychain, Linux Secret Service, or Windows Credential Manager permissions."
                        .to_string(),
                },
            ],
        }
    }

    fn invalid(message: String) -> Self {
        Self {
            code: "identity_invalid_crypto_material",
            boundary: "SovereignIdentity",
            message,
            recovery_options: Vec::new(),
        }
    }

    fn integrity(message: &str) -> Self {
        Self {
            code: "ledger_integrity_violation",
            boundary: "SovereignIdentity",
            message: message.to_string(),
            recovery_options: Vec::new(),
        }
    }
}

#[cfg(test)]
#[path = "sovereign_identity_security_tests.rs"]
// Keep session-key and profile-integrity regressions together while the runtime
// identity implementation remains within its reviewed source-size boundary.
mod security_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_verification_does_not_read_keychain_after_cache_eviction() {
        let root = std::env::temp_dir().join(format!(
            "oomu_identity_public_verify_{}_{}",
            std::process::id(),
            unix_time_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let profile_path = root.join(IDENTITY_PROFILE);
        let signing_key = SigningKey::generate(&mut OsRng);
        let public_key = hex::encode(signing_key.verifying_key().to_bytes());
        std::fs::write(
            &profile_path,
            serde_json::json!({ "public_key": public_key }).to_string(),
        )
        .unwrap();
        let identity = SovereignIdentity::new_without_test_override(profile_path);
        let payload = "verify from the public profile only";
        let signature = signature_block_from_signing_key(&signing_key, payload);

        let (service, account) = keychain_location();
        crate::keychain_session::evict_for_test(service, account);
        let reads_before = crate::keychain_session::backend_read_count_for_test(service, account);
        identity.verify_payload(payload, &signature).unwrap();
        assert_eq!(
            crate::keychain_session::backend_read_count_for_test(service, account),
            reads_before
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn generated_node_identity_uses_root_derived_key_without_plaintext_file() {
        let _env_guard = APP_DATA_ENV_LOCK.lock().unwrap();
        let root = std::env::temp_dir().join(format!(
            "oomu_node_identity_derived_{}_{}",
            std::process::id(),
            unix_time_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let previous_app_root = std::env::var_os(crate::settings::APP_DATA_ROOT_ENV);
        std::env::set_var(crate::settings::APP_DATA_ROOT_ENV, &root);

        let identity = SovereignIdentity::initialize_ephemeral();
        let profile = identity
            .generate_node_identity()
            .expect("node identity generates");

        assert_eq!(profile.private_key_path, node_private_key_location());
        assert!(!root
            .join(OOMU_IDENTITY_DIR)
            .join(LEGACY_NODE_PRIVATE_KEY_FILE)
            .exists());
        identity
            .sign_node_payload("node payload")
            .expect("node payload signs from test node seed");

        if let Some(previous_app_root) = previous_app_root {
            std::env::set_var(crate::settings::APP_DATA_ROOT_ENV, previous_app_root);
        } else {
            std::env::remove_var(crate::settings::APP_DATA_ROOT_ENV);
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn legacy_node_key_is_retired_and_replaced_by_root_derived_profile() {
        let _env_guard = APP_DATA_ENV_LOCK.lock().unwrap();
        let root = std::env::temp_dir().join(format!(
            "oomu_node_identity_migration_{}_{}",
            std::process::id(),
            unix_time_ms()
        ));
        let identity_dir = root.join(OOMU_IDENTITY_DIR);
        std::fs::create_dir_all(&identity_dir).unwrap();
        let previous_app_root = std::env::var_os(crate::settings::APP_DATA_ROOT_ENV);
        std::env::set_var(crate::settings::APP_DATA_ROOT_ENV, &root);

        let identity = SovereignIdentity::initialize_ephemeral();
        let signing_key = SigningKey::generate(&mut OsRng);
        let public_key = hex::encode(signing_key.verifying_key().to_bytes());
        let node_id = "oomu-node-legacy-test".to_string();
        let created_at_ms = unix_time_ms();
        let profile = NodeIdentityProfile {
            node_id: node_id.clone(),
            public_key: public_key.clone(),
            public_key_fingerprint: fingerprint(&public_key),
            private_key_path: identity_dir
                .join(LEGACY_NODE_PRIVATE_KEY_FILE)
                .to_string_lossy()
                .to_string(),
            identity_dir: identity_dir.to_string_lossy().to_string(),
            architect_signature: identity
                .sign_payload(&node_identity_payload(&node_id, &public_key, created_at_ms))
                .expect("architect proof signs"),
            created_at_ms,
        };
        std::fs::write(
            identity_dir.join(NODE_IDENTITY_FILE),
            serde_json::to_string_pretty(&profile).unwrap(),
        )
        .unwrap();
        std::fs::write(
            identity_dir.join(LEGACY_NODE_PRIVATE_KEY_FILE),
            hex::encode(signing_key.to_bytes()),
        )
        .unwrap();

        identity
            .retire_legacy_local_node_key()
            .expect("legacy key is securely retired");

        assert!(!identity_dir.join(LEGACY_NODE_PRIVATE_KEY_FILE).exists());
        let replacement = identity
            .generate_node_identity()
            .expect("derived node profile replaces legacy profile");
        assert_eq!(replacement.private_key_path, node_private_key_location());
        assert_ne!(replacement.public_key, public_key);

        if let Some(previous_app_root) = previous_app_root {
            std::env::set_var(crate::settings::APP_DATA_ROOT_ENV, previous_app_root);
        } else {
            std::env::remove_var(crate::settings::APP_DATA_ROOT_ENV);
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn persistent_node_derivation_is_stable_and_domain_separated() {
        let root_seed = "11".repeat(32);
        let first = derive_persistent_node_seed(&root_seed).expect("node seed derives");
        let second = derive_persistent_node_seed(&root_seed).expect("node seed re-derives");
        assert_eq!(first, second);
        assert_ne!(hex::encode(first), root_seed);
    }
}
