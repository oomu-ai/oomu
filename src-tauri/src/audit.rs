use crate::{
    db::PersistenceEngine, foundation::digest::sha256_hex, knowledge::KnowledgeStore,
    sovereign_identity::SovereignIdentity,
};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

const RELEASE_VERSION: &str = env!("OOMU_RELEASE_VERSION");
const RELEASE_BUILD_NUMBER: &str = env!("OOMU_RELEASE_BUILD_NUMBER");
const RELEASE_DIR: &str = "release/pre_alpha";
const AUDIT_REPORT_FILE: &str = "audit_024_report.json";
const MISSION_CHRONICLE_FILE: &str = "mission_chronicle_024.json";
const RELEASE_GATE_FILE: &str = "release-gate.json";
const REQUIRED_RUNS: usize = 3;
// Reviewed Architect Root release key. Rotation is a source-reviewed operation and requires
// coordinated provisioning of the matching private key in the release secret store.
const TRUSTED_RELEASE_PUBLIC_KEY_HEX: &str =
    "10543bcbfa20b4c58d587aa969053124cc3340b11470b84ba9df763fee9100bb";
const REQUIRED_RELEASE_CHECKS: [&str; 13] = [
    "apple_toolchain",
    "dependency_audit",
    "pdf_containment",
    "automated_tests",
    "release_sanitizer",
    "database_sanitizer",
    "entitlement_snapshot",
    "artifact_validation",
    "signing",
    "notarization",
    "stapling",
    "manifest_verification",
    "clean_machine_launch",
];

#[derive(Clone)]
pub struct PreAlphaAudit {
    release_dir: Arc<PathBuf>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PreAlphaAuditRequest {
    pub runs: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreAlphaAuditReport {
    pub version: String,
    pub status: String,
    pub runs_required: usize,
    pub runs_completed: usize,
    pub success_rate: f64,
    pub state_drift_detected: bool,
    pub unhandled_exceptions: i64,
    pub mean_mission_ms: f64,
    pub topology_snapshot_hash: String,
    pub final_witness_hash: String,
    pub report_path: String,
    pub mission_chronicle_path: String,
    pub release_dir: String,
    pub launch_readiness: LaunchReadiness,
    pub runs: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LaunchReadiness {
    pub version: String,
    pub build_number: String,
    pub strict_mlc_mode: bool,
    pub sanitizer_enabled: bool,
    pub database_sanitized: bool,
    pub manifest_verified: bool,
    pub release_gate_passed: bool,
    pub build_identifier: Option<String>,
    pub artifact_digest: Option<String>,
    pub telemetry_ready: bool,
    pub release_assets_staged: bool,
    pub docs_frozen: bool,
    pub idle_memory_target_mb: i64,
    pub launch_target_ms: i64,
    pub audit_report_path: String,
}

#[derive(Debug, Deserialize)]
struct SignedReleaseEvidenceGate {
    schema_version: u64,
    kind: String,
    payload_json: String,
    payload_sha256: String,
    signature: ReleaseGateSignature,
}

#[derive(Debug, Deserialize)]
struct ReleaseGateSignature {
    algorithm: String,
    public_key_hex: String,
    key_fingerprint_sha256: String,
    value_base64: String,
}

#[derive(Debug, Deserialize)]
struct ReleaseEvidenceGate {
    schema_version: u64,
    kind: String,
    status: String,
    synthetic: bool,
    strict_mlc_mode: bool,
    build_identifier: String,
    source_revision: String,
    artifact_identifier: String,
    artifact_digest: String,
    verified_at: String,
    expires_at: String,
    checks: Vec<ReleaseEvidenceCheck>,
}

#[derive(Debug, Deserialize)]
struct ReleaseEvidenceCheck {
    evidence_type: String,
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct ExecutedEvidenceRecord {
    schema_version: u64,
    kind: String,
    evidence_type: String,
    status: String,
    synthetic: bool,
    build_identifier: String,
    source_revision: String,
    artifact_identifier: String,
    artifact_digest: String,
    produced_at: String,
    expires_at: String,
    execution: ExecutedEvidenceExecution,
}

#[derive(Debug, Deserialize)]
struct ExecutedEvidenceExecution {
    executed: bool,
    exit_code: i64,
}

#[derive(Debug, Serialize)]
pub struct AuditError {
    pub code: &'static str,
    pub boundary: &'static str,
    pub message: String,
}

impl PreAlphaAudit {
    pub fn initialize() -> Result<Self, String> {
        let release_dir = std::env::var_os("OOMU_RELEASE_EVIDENCE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| project_root().join(RELEASE_DIR));
        Self::initialize_at(release_dir)
    }

    pub(crate) fn initialize_at(release_dir: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&release_dir).map_err(|error| error.to_string())?;
        Ok(Self {
            release_dir: Arc::new(release_dir),
        })
    }

    async fn run_full_audit(
        &self,
        request: PreAlphaAuditRequest,
        knowledge: KnowledgeStore,
        identity: SovereignIdentity,
    ) -> Result<PreAlphaAuditReport, AuditError> {
        let audit = self.clone();
        tauri::async_runtime::spawn_blocking(move || {
            audit.run_full_audit_sync(request, knowledge, identity)
        })
        .await
        .map_err(|error| AuditError::runtime(error.to_string()))?
    }

    fn run_full_audit_sync(
        &self,
        request: PreAlphaAuditRequest,
        _knowledge: KnowledgeStore,
        _identity: SovereignIdentity,
    ) -> Result<PreAlphaAuditReport, AuditError> {
        fs::create_dir_all(&*self.release_dir).map_err(AuditError::io)?;
        let runs_required = request.runs.unwrap_or(REQUIRED_RUNS).clamp(1, 9);
        let launch_readiness = self.launch_readiness_sync();
        let topology_snapshot_hash = hash_text(
            &serde_json::json!({
                "build_identifier": launch_readiness.build_identifier,
                "artifact_digest": launch_readiness.artifact_digest,
                "executed_mission_runs": 0
            })
            .to_string(),
        );
        let report = PreAlphaAuditReport {
            version: RELEASE_VERSION.to_string(),
            status: "attention_required".to_string(),
            runs_required,
            runs_completed: 0,
            success_rate: 0.0,
            state_drift_detected: false,
            unhandled_exceptions: runs_required as i64,
            mean_mission_ms: 0.0,
            topology_snapshot_hash,
            final_witness_hash: hash_text("no-external-mission-execution-records"),
            report_path: self
                .release_dir
                .join(AUDIT_REPORT_FILE)
                .to_string_lossy()
                .to_string(),
            mission_chronicle_path: self
                .release_dir
                .join(MISSION_CHRONICLE_FILE)
                .to_string_lossy()
                .to_string(),
            release_dir: self.release_dir.to_string_lossy().to_string(),
            launch_readiness,
            runs: Vec::new(),
        };
        let chronicle = serde_json::json!({
            "version": RELEASE_VERSION,
            "status": "not_executed",
            "synthetic": false,
            "requested_runs": runs_required,
            "completed_runs": 0,
            "actions": [],
            "reason": "No externally executed mission record was supplied; self-authored checkpoints are prohibited."
        });
        write_json(&self.release_dir.join(MISSION_CHRONICLE_FILE), &chronicle)?;
        write_json(&self.release_dir.join(AUDIT_REPORT_FILE), &report)?;
        Ok(report)
    }

    fn launch_readiness_sync(&self) -> LaunchReadiness {
        let docs_frozen = self.release_dir.join("USER_MANUAL.md").exists()
            && self.release_dir.join("API_REFERENCE.md").exists();
        let gate = self.current_release_gate();
        let has_check = |evidence_type: &str| {
            gate.as_ref().is_some_and(|gate| {
                gate.checks
                    .iter()
                    .any(|check| check.evidence_type == evidence_type)
            })
        };
        LaunchReadiness {
            version: RELEASE_VERSION.to_string(),
            build_number: RELEASE_BUILD_NUMBER.to_string(),
            strict_mlc_mode: gate.as_ref().is_some_and(|gate| gate.strict_mlc_mode),
            sanitizer_enabled: has_check("release_sanitizer"),
            database_sanitized: has_check("database_sanitizer"),
            manifest_verified: has_check("manifest_verification"),
            release_gate_passed: gate.is_some(),
            build_identifier: gate.as_ref().map(|gate| gate.build_identifier.clone()),
            artifact_digest: gate.as_ref().map(|gate| gate.artifact_digest.clone()),
            telemetry_ready: home_dir()
                .join(".oomu/vault/telemetry/stress_020.json")
                .exists(),
            release_assets_staged: has_check("artifact_validation"),
            docs_frozen,
            idle_memory_target_mb: 400,
            launch_target_ms: 1500,
            audit_report_path: self
                .release_dir
                .join(AUDIT_REPORT_FILE)
                .to_string_lossy()
                .to_string(),
        }
    }

    async fn launch_readiness(&self) -> Result<LaunchReadiness, AuditError> {
        Ok(self.launch_readiness_sync())
    }

    fn current_release_gate(&self) -> Option<ReleaseEvidenceGate> {
        let gate_path = self.release_dir.join(RELEASE_GATE_FILE);
        if !is_immutable_regular_file(&gate_path) {
            return None;
        }
        let signed_gate: SignedReleaseEvidenceGate =
            serde_json::from_slice(&fs::read(gate_path).ok()?).ok()?;
        let trusted_bytes = hex::decode(TRUSTED_RELEASE_PUBLIC_KEY_HEX).ok()?;
        let trusted_public_key: [u8; 32] = trusted_bytes.try_into().ok()?;
        self.verify_signed_release_gate(signed_gate, &trusted_public_key)
    }

    fn verify_signed_release_gate(
        &self,
        signed_gate: SignedReleaseEvidenceGate,
        trusted_public_key: &[u8; 32],
    ) -> Option<ReleaseEvidenceGate> {
        if signed_gate.schema_version != 1
            || signed_gate.kind != "oomu.signed-release-evidence-gate"
            || signed_gate.signature.algorithm != "ed25519"
            || signed_gate.signature.public_key_hex != hex::encode(trusted_public_key)
            || signed_gate.signature.key_fingerprint_sha256 != hash_bytes(trusted_public_key)
            || signed_gate.payload_sha256 != hash_text(&signed_gate.payload_json)
        {
            return None;
        }
        let verifying_key = VerifyingKey::from_bytes(trusted_public_key).ok()?;
        let signature = Signature::from_slice(
            &BASE64_STANDARD
                .decode(signed_gate.signature.value_base64.as_bytes())
                .ok()?,
        )
        .ok()?;
        verifying_key
            .verify(signed_gate.payload_json.as_bytes(), &signature)
            .ok()?;

        let gate: ReleaseEvidenceGate = serde_json::from_str(&signed_gate.payload_json).ok()?;
        if gate.schema_version != 1
            || gate.kind != "oomu.release-evidence-gate"
            || gate.status != "passed"
            || gate.synthetic
            || !gate.strict_mlc_mode
            || gate.build_identifier.trim().is_empty()
            || gate.artifact_identifier.trim().is_empty()
            || !is_sha256_digest(&gate.artifact_digest)
            || !is_git_revision(&gate.source_revision)
        {
            return None;
        }
        let verified_at = chrono::DateTime::parse_from_rfc3339(&gate.verified_at).ok()?;
        let expires_at = chrono::DateTime::parse_from_rfc3339(&gate.expires_at).ok()?;
        let now = chrono::Utc::now();
        if verified_at > now + chrono::Duration::minutes(1) || expires_at <= now {
            return None;
        }

        let mut seen = HashSet::new();
        let mut minimum_expiration: Option<chrono::DateTime<chrono::FixedOffset>> = None;
        for check in &gate.checks {
            if !REQUIRED_RELEASE_CHECKS.contains(&check.evidence_type.as_str())
                || !seen.insert(check.evidence_type.as_str())
                || !is_plain_sha256(&check.sha256)
            {
                return None;
            }
            let evidence_path = self
                .release_dir
                .join(format!("{}.json", check.evidence_type));
            if !is_immutable_regular_file(&evidence_path) {
                return None;
            }
            let evidence_bytes = fs::read(evidence_path).ok()?;
            if hash_bytes(&evidence_bytes) != check.sha256 {
                return None;
            }
            let evidence: ExecutedEvidenceRecord = serde_json::from_slice(&evidence_bytes).ok()?;
            let produced_at = chrono::DateTime::parse_from_rfc3339(&evidence.produced_at).ok()?;
            let evidence_expires_at =
                chrono::DateTime::parse_from_rfc3339(&evidence.expires_at).ok()?;
            let maximum_freshness = evidence_freshness(&check.evidence_type)?;
            if evidence.schema_version != 1
                || evidence.kind != "oomu.executed-release-evidence"
                || evidence.evidence_type != check.evidence_type
                || evidence.status != "passed"
                || evidence.synthetic
                || !evidence.execution.executed
                || evidence.execution.exit_code != 0
                || evidence.build_identifier != gate.build_identifier
                || evidence.source_revision != gate.source_revision
                || evidence.artifact_identifier != gate.artifact_identifier
                || evidence.artifact_digest != gate.artifact_digest
                || produced_at > now + chrono::Duration::minutes(1)
                || evidence_expires_at <= now
                || evidence_expires_at <= produced_at
                || evidence_expires_at - produced_at > maximum_freshness
            {
                return None;
            }
            minimum_expiration = Some(minimum_expiration.map_or(evidence_expires_at, |current| {
                current.min(evidence_expires_at)
            }));
        }
        if seen.len() != REQUIRED_RELEASE_CHECKS.len()
            || REQUIRED_RELEASE_CHECKS
                .iter()
                .any(|required| !seen.contains(required))
            || minimum_expiration
                .map(|minimum| expires_at > minimum)
                .unwrap_or(true)
        {
            return None;
        }
        Some(gate)
    }
}

#[tauri::command]
pub async fn run_pre_alpha_audit(
    request: PreAlphaAuditRequest,
    audit: tauri::State<'_, PreAlphaAudit>,
    knowledge: tauri::State<'_, KnowledgeStore>,
    identity: tauri::State<'_, SovereignIdentity>,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<PreAlphaAuditReport, AuditError> {
    persistence
        .require_durable_store("pre-alpha release audit")
        .map_err(AuditError::runtime)?;
    audit
        .run_full_audit(request, knowledge.inner().clone(), identity.inner().clone())
        .await
}

#[tauri::command]
pub async fn get_launch_readiness(
    audit: tauri::State<'_, PreAlphaAudit>,
) -> Result<LaunchReadiness, AuditError> {
    audit.launch_readiness().await
}

fn evidence_freshness(evidence_type: &str) -> Option<chrono::Duration> {
    if evidence_type == "clean_machine_launch" {
        Some(chrono::Duration::days(7))
    } else if REQUIRED_RELEASE_CHECKS.contains(&evidence_type) {
        Some(chrono::Duration::hours(24))
    } else {
        None
    }
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), AuditError> {
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|error| AuditError::runtime(error.to_string()))?;
    fs::write(path, bytes).map_err(AuditError::io)
}

fn hash_text(value: &str) -> String {
    hash_bytes(value.as_bytes())
}

fn hash_bytes(value: &[u8]) -> String {
    sha256_hex(value)
}

fn is_plain_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_sha256_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(is_plain_sha256)
}

fn is_git_revision(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_immutable_regular_file(path: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o222 != 0 {
            return false;
        }
    }
    true
}

fn project_root() -> PathBuf {
    crate::settings::app_data_root()
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(project_root)
}

impl AuditError {
    fn io(error: std::io::Error) -> Self {
        Self {
            code: "pre_alpha_audit_io",
            boundary: "release/pre_alpha",
            message: error.to_string(),
        }
    }

    fn runtime(message: String) -> Self {
        Self {
            code: "pre_alpha_audit_runtime",
            boundary: "audit.rs",
            message,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn test_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "oomu-audit-{label}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn make_immutable(path: &Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o444)).unwrap();
        }
    }

    fn write_evidence_records(
        root: &Path,
        now: chrono::DateTime<chrono::Utc>,
        first_expiration: chrono::DateTime<chrono::Utc>,
    ) -> Vec<serde_json::Value> {
        REQUIRED_RELEASE_CHECKS
            .iter()
            .enumerate()
            .map(|(index, evidence_type)| {
                let produced_at = if *evidence_type == "clean_machine_launch" {
                    first_expiration - chrono::Duration::days(7)
                } else if index == 0 {
                    first_expiration - chrono::Duration::hours(24)
                } else {
                    now
                };
                let expires_at = if *evidence_type == "clean_machine_launch" {
                    produced_at + chrono::Duration::days(7)
                } else {
                    produced_at + chrono::Duration::hours(24)
                };
                let result = serde_json::json!({ "passed": true });
                let record = serde_json::json!({
                    "schema_version": 1,
                    "kind": "oomu.executed-release-evidence",
                    "evidence_type": evidence_type,
                    "status": "passed",
                    "synthetic": false,
                    "build_identifier": "build-214",
                    "source_revision": "a".repeat(40),
                    "artifact_identifier": "oomu-macos-build-214",
                    "artifact_digest": format!("sha256:{}", "b".repeat(64)),
                    "produced_at": produced_at.to_rfc3339(),
                    "expires_at": expires_at.to_rfc3339(),
                    "producer": {
                        "executable": "/usr/bin/true",
                        "component": evidence_type,
                        "endpoint": "test",
                        "input": "test"
                    },
                    "execution": { "executed": true, "exit_code": 0 },
                    "result": result
                });
                let bytes = serde_json::to_vec(&record).unwrap();
                let path = root.join(format!("{evidence_type}.json"));
                fs::write(&path, &bytes).unwrap();
                make_immutable(&path);
                serde_json::json!({
                    "evidence_type": evidence_type,
                    "sha256": hash_bytes(&bytes)
                })
            })
            .collect()
    }

    fn signed_gate(
        checks: Vec<serde_json::Value>,
        signing_key: &SigningKey,
        now: chrono::DateTime<chrono::Utc>,
        expires_at: chrono::DateTime<chrono::Utc>,
        synthetic: bool,
    ) -> SignedReleaseEvidenceGate {
        let payload_json = serde_json::json!({
            "schema_version": 1,
            "kind": "oomu.release-evidence-gate",
            "status": "passed",
            "synthetic": synthetic,
            "strict_mlc_mode": true,
            "build_identifier": "build-214",
            "source_revision": "a".repeat(40),
            "artifact_identifier": "oomu-macos-build-214",
            "artifact_digest": format!("sha256:{}", "b".repeat(64)),
            "verified_at": now.to_rfc3339(),
            "expires_at": expires_at.to_rfc3339(),
            "checks": checks
        })
        .to_string();
        let public_key = signing_key.verifying_key().to_bytes();
        SignedReleaseEvidenceGate {
            schema_version: 1,
            kind: "oomu.signed-release-evidence-gate".to_string(),
            payload_sha256: hash_text(&payload_json),
            signature: ReleaseGateSignature {
                algorithm: "ed25519".to_string(),
                public_key_hex: hex::encode(public_key),
                key_fingerprint_sha256: hash_bytes(&public_key),
                value_base64: BASE64_STANDARD
                    .encode(signing_key.sign(payload_json.as_bytes()).to_bytes()),
            },
            payload_json,
        }
    }

    #[test]
    fn pre_alpha_audit_cannot_generate_passing_mission_evidence() {
        let root = test_root("no-synthetic-missions");
        let audit = PreAlphaAudit::initialize_at(root.clone()).unwrap();
        let knowledge = KnowledgeStore::initialize_at(root.join("knowledge.db")).unwrap();
        let report = audit
            .run_full_audit_sync(
                PreAlphaAuditRequest { runs: Some(3) },
                knowledge,
                SovereignIdentity::initialize_ephemeral(),
            )
            .unwrap();
        assert_eq!(report.status, "attention_required");
        assert_eq!(report.runs_completed, 0);
        assert_eq!(report.unhandled_exceptions, 3);
        assert!(report.runs.is_empty());
        let chronicle = fs::read_to_string(root.join(MISSION_CHRONICLE_FILE)).unwrap();
        assert!(chronicle.contains("not_executed"));
        assert!(!chronicle.contains("model_switch"));
        assert!(!chronicle.contains("\"status\": \"verified\""));
    }

    #[test]
    fn signed_gate_rejects_forgery_synthetic_state_and_expiration_extension() {
        let root = test_root("signed-gate");
        fs::create_dir_all(&root).unwrap();
        let audit = PreAlphaAudit::initialize_at(root.clone()).unwrap();
        let now = chrono::Utc::now();
        let near_expiration = now + chrono::Duration::minutes(2);
        let checks = write_evidence_records(&root, now, near_expiration);
        let trusted_key = SigningKey::from_bytes(&[7_u8; 32]);
        let trusted_public = trusted_key.verifying_key().to_bytes();

        let extended = signed_gate(
            checks.clone(),
            &trusted_key,
            now,
            now + chrono::Duration::hours(24),
            false,
        );
        assert!(audit
            .verify_signed_release_gate(extended, &trusted_public)
            .is_none());

        let synthetic = signed_gate(checks.clone(), &trusted_key, now, near_expiration, true);
        assert!(audit
            .verify_signed_release_gate(synthetic, &trusted_public)
            .is_none());

        let attacker_key = SigningKey::from_bytes(&[9_u8; 32]);
        let forged = signed_gate(checks.clone(), &attacker_key, now, near_expiration, false);
        assert!(audit
            .verify_signed_release_gate(forged, &trusted_public)
            .is_none());

        let valid = signed_gate(checks, &trusted_key, now, near_expiration, false);
        assert!(audit
            .verify_signed_release_gate(valid, &trusted_public)
            .is_some());
    }

    #[test]
    fn unsigned_handwritten_gate_never_satisfies_launch_readiness() {
        let root = test_root("unsigned-gate");
        let audit = PreAlphaAudit::initialize_at(root.clone()).unwrap();
        let path = root.join(RELEASE_GATE_FILE);
        fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({
                "kind": "oomu.release-evidence-gate",
                "status": "passed",
                "checks": REQUIRED_RELEASE_CHECKS
            }))
            .unwrap(),
        )
        .unwrap();
        make_immutable(&path);
        assert!(!audit.launch_readiness_sync().release_gate_passed);
    }
}
