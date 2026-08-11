use crate::agentic_loop::{step_to_request, ActionPlan};
use crate::foundation::{
    clock::unix_time_ms_u128 as unix_time_ms,
    digest::{sha256_file_hex, sha256_hex},
};
use crate::shield_gate::{
    authorize_action, authorize_action_for_approved_plan,
    validate_logical_certificate_for_host_access, AuthorizedActions, LogicalCertificate,
};
use crate::sovereign_identity::{SignatureBlock, SovereignIdentity};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

mod artifact_claim;
pub(crate) mod native_terminal_claim;
mod verified_evidence_claims;

const AUDIT_DB_PATH: &str = "release/pre_alpha/audit_024.sqlite";

#[derive(Debug, Clone, Serialize)]
pub struct VerificationReport {
    pub verified: bool,
    pub log_path: String,
    pub claims_checked: usize,
    pub failures: Vec<String>,
}

#[derive(Debug)]
pub struct VerificationError {
    pub message: String,
    pub log_path: Option<String>,
}

pub struct MlcVerifier {
    root: PathBuf,
}

#[derive(Debug)]
pub(crate) struct PlanVerificationReport {
    pub authorized_actions: Vec<AuthorizedActions>,
    pub execution_path: Vec<String>,
}

impl MlcVerifier {
    pub fn new() -> Self {
        Self {
            root: project_root(),
        }
    }

    pub(crate) fn verify_with_identity(
        &self,
        mlc_path: &str,
        identity: &SovereignIdentity,
    ) -> Result<VerificationReport, VerificationError> {
        let path = PathBuf::from(mlc_path);
        if !path.starts_with(&self.root) {
            return Err(VerificationError {
                message: "MLC path is outside the project quarantine.".to_string(),
                log_path: None,
            });
        }

        let content = fs::read_to_string(&path).map_err(|error| VerificationError {
            message: format!("Failed to read MLC {}: {error}", path.display()),
            log_path: None,
        })?;
        let claims = extract_claims(&content);
        let mut failures = Vec::new();
        if claims.is_empty() {
            failures.push("MLC contains no deterministic execution claims.".to_string());
        }

        for claim in &claims {
            if let Err(message) = self.verify_claim_with_identity(claim, identity) {
                failures.push(message);
            }
        }

        let verified = failures.is_empty();
        let report = VerificationReport {
            verified,
            log_path: self.write_verifier_log(mlc_path, &claims, &failures)?,
            claims_checked: claims.len(),
            failures,
        };

        if report.verified {
            Ok(report)
        } else {
            Err(VerificationError {
                message: format!(
                    "MLC verification failed for {} claim(s).",
                    report.failures.len()
                ),
                log_path: Some(report.log_path),
            })
        }
    }

    pub(crate) fn verify_plan(
        &self,
        plan: &ActionPlan,
        identity: &SovereignIdentity,
    ) -> Result<PlanVerificationReport, VerificationError> {
        self.verify_plan_with_authorization(plan, identity, false)
    }

    pub(crate) fn verify_plan_preview(
        &self,
        plan: &ActionPlan,
        identity: &SovereignIdentity,
    ) -> Result<PlanVerificationReport, VerificationError> {
        self.verify_plan_with_authorization(plan, identity, true)
    }

    pub(crate) fn verify_approved_plan(
        &self,
        plan: &ActionPlan,
        identity: &SovereignIdentity,
    ) -> Result<PlanVerificationReport, VerificationError> {
        self.verify_approved_plan_from_step(plan, identity, 0)
    }

    pub(crate) fn verify_approved_plan_from_step(
        &self,
        plan: &ActionPlan,
        identity: &SovereignIdentity,
        first_uncompleted_step: usize,
    ) -> Result<PlanVerificationReport, VerificationError> {
        self.verify_plan_with_authorization_from_step(plan, identity, true, first_uncompleted_step)
    }

    fn verify_plan_with_authorization(
        &self,
        plan: &ActionPlan,
        identity: &SovereignIdentity,
        plan_approved: bool,
    ) -> Result<PlanVerificationReport, VerificationError> {
        self.verify_plan_with_authorization_from_step(plan, identity, plan_approved, 0)
    }

    fn verify_plan_with_authorization_from_step(
        &self,
        plan: &ActionPlan,
        identity: &SovereignIdentity,
        plan_approved: bool,
        first_uncompleted_step: usize,
    ) -> Result<PlanVerificationReport, VerificationError> {
        let mut failures = Vec::new();
        let mut authorized_actions =
            Vec::with_capacity(plan.steps.len().saturating_sub(first_uncompleted_step));
        let mut execution_path = Vec::new();
        let certificate = &plan.logical_certificate;

        if first_uncompleted_step > plan.steps.len() {
            failures.push(
                "The verified execution checkpoint is beyond the signed ActionPlan.".to_string(),
            );
        }

        if let Err(error) =
            validate_logical_certificate_for_host_access("action_plan", Some(certificate), identity)
        {
            failures.push(error.message);
        }

        let expected_premises = expected_plan_premises(plan);
        if !certificate.premises.starts_with(&expected_premises) {
            failures.push(
                "Logical Certificate premises do not begin with the ActionPlan objective and plan ID."
                    .to_string(),
            );
        }

        let expected_execution_path = expected_plan_execution_path(plan);
        if certificate.execution_path != expected_execution_path {
            failures.push(
                "Logical Certificate execution_path does not exactly match the ActionPlan steps."
                    .to_string(),
            );
        }

        if certificate.formal_conclusion != plan.exit_condition {
            failures.push(
                "Logical Certificate formal_conclusion does not match the ActionPlan exit condition."
                    .to_string(),
            );
        }

        for step in plan.steps.iter().skip(first_uncompleted_step) {
            let authorization = if plan_approved {
                authorize_action_for_approved_plan(step_to_request(step))
            } else {
                authorize_action(step_to_request(step))
            };
            match authorization {
                Ok(action) => {
                    execution_path.push(format!(
                        "Authorized step '{}' at {:?} risk.",
                        step.step, step.risk_level
                    ));
                    authorized_actions.push(action);
                }
                Err(error) => {
                    failures.push(format!(
                        "Shield Gate rejected step '{}': {}",
                        step.step, error.message
                    ));
                }
            }
        }

        if failures.is_empty() {
            return Ok(PlanVerificationReport {
                authorized_actions,
                execution_path,
            });
        }

        let log_path = self.write_plan_verifier_log(plan, &failures).ok();
        Err(VerificationError {
            message: preflight_failure_message(&failures),
            log_path,
        })
    }

    fn verify_operation_claim(
        &self,
        claim: &str,
        identity: &SovereignIdentity,
    ) -> Result<(), String> {
        let op = claim_value(claim, "operation")
            .ok_or_else(|| format!("Missing operation in claim: {claim}"))?;
        let status = claim_value(claim, "status")
            .ok_or_else(|| format!("Missing status in claim: {claim}"))?;
        if status != "completed" {
            return Err(format!("Operation {op} status is not completed: {status}"));
        }
        verify_semantic_reasoning_claim(op, claim)?;

        let node_id = claim_value(claim, "node_id")
            .ok_or_else(|| format!("Missing node_id in claim: {claim}"))?;
        let hash =
            claim_value(claim, "hash").ok_or_else(|| format!("Missing hash in claim: {claim}"))?;
        let sig_json_raw = claim_value(claim, "signature_json")
            .ok_or_else(|| format!("Missing signature_json in claim: {claim}"))?;

        let signature_block: SignatureBlock = serde_json::from_str(sig_json_raw)
            .map_err(|err| format!("Invalid signature_json in claim: {err}"))?;

        let local_node = identity
            .node_identity()
            .map_err(|error| format!("Failed to load local node identity: {}", error.message))?;
        if local_node.node_id != node_id {
            return Err(format!(
                "Remote mesh node signatures are no longer accepted after local-only purge: {node_id}"
            ));
        }
        let expected_public_key = local_node.public_key;

        if signature_block.public_key != expected_public_key {
            return Err(format!(
                "Signature public key mismatch for node {}: claim public key {}, registered public key {}",
                node_id, signature_block.public_key, expected_public_key
            ));
        }

        identity
            .verify_node_payload(hash, &signature_block)
            .map_err(|err| format!("Operation signature verification failed: {}", err.message))?;

        Ok(())
    }

    fn verify_claim_with_identity(
        &self,
        claim: &str,
        identity: &SovereignIdentity,
    ) -> Result<(), String> {
        if claim.starts_with("operation=") {
            return self.verify_operation_claim(claim, identity);
        }

        if claim.starts_with("local_certificate_hash=") {
            return self.verify_local_certificate_claim(claim, identity);
        }

        if claim.starts_with("native_terminal_receipt ") {
            return native_terminal_claim::verify(claim);
        }

        if claim.starts_with("state_resumed ") {
            return self.verify_state_resumed_claim(claim, identity);
        }

        if claim.starts_with("connector_task_evidence ") {
            for field in ["result_sha256", "citation_sha256"] {
                let digest = claim_value(claim, field)
                    .ok_or_else(|| format!("Missing {field} in connector evidence claim."))?;
                if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return Err(format!(
                        "Connector evidence {field} is not a SHA-256 digest."
                    ));
                }
            }
            if claim_value(claim, "evidence_recorded") != Some("true")
                || !matches!(
                    claim_value(claim, "postcondition_recorded"),
                    Some("true" | "false")
                )
            {
                return Err("Connector evidence claim has invalid recording flags.".to_string());
            }
            return Ok(());
        }

        if claim.starts_with("file_exists ") {
            let path =
                claim_path_value(claim).ok_or_else(|| format!("Missing path in claim: {claim}"))?;
            let min_bytes = claim_value(claim, "min_bytes")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0);
            let path = PathBuf::from(path);
            self.guard_claim_path(&path)?;
            let metadata = fs::metadata(&path)
                .map_err(|error| format!("File claim failed for {}: {error}", path.display()))?;

            if !metadata.is_file() {
                return Err(format!("Claimed file is not a file: {}", path.display()));
            }

            if metadata.len() < min_bytes {
                return Err(format!(
                    "Claimed file {} has {} bytes, expected at least {min_bytes}.",
                    path.display(),
                    metadata.len()
                ));
            }

            return Ok(());
        }

        if claim.starts_with("dir_exists ") {
            let path =
                claim_path_value(claim).ok_or_else(|| format!("Missing path in claim: {claim}"))?;
            let path = PathBuf::from(path);
            self.guard_claim_path(&path)?;
            let metadata = fs::metadata(&path).map_err(|error| {
                format!("Directory claim failed for {}: {error}", path.display())
            })?;

            if metadata.is_dir() {
                return Ok(());
            }

            return Err(format!(
                "Claimed directory is not a directory: {}",
                path.display()
            ));
        }

        if claim.starts_with("directory_entries ") {
            return self.verify_directory_entries_claim(claim);
        }

        if claim.starts_with("shield_gate_approved_external_write ") {
            return self.verify_approved_external_write_claim(claim);
        }

        if claim.starts_with("local_file_created ") {
            return self.verify_local_file_created_claim(claim);
        }

        if claim.starts_with("artifact_verified=") {
            return self.verify_artifact_claim(claim);
        }

        if let Some(result) = artifact_claim::verify(claim) {
            return result;
        }

        if let Some(result) = verified_evidence_claims::verify(claim) {
            return result;
        }

        Err(format!("Unknown MLC claim: {claim}"))
    }

    #[cfg(test)]
    fn verify_claim(&self, claim: &str) -> Result<(), String> {
        let identity = SovereignIdentity::initialize()
            .map_err(|error| format!("Failed to initialize SovereignIdentity: {error}"))?;
        self.verify_claim_with_identity(claim, &identity)
    }

    fn verify_local_file_created_claim(&self, claim: &str) -> Result<(), String> {
        let format = claim_value(claim, "format")
            .ok_or_else(|| format!("Missing format in local_file_created claim: {claim}"))?;
        if format.is_empty()
            || format.len() > 16
            || !format
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        {
            return Err(
                "local_file_created format must be a short lowercase file extension.".to_string(),
            );
        }

        let claimed_sha256 = claim_value(claim, "sha256")
            .ok_or_else(|| format!("Missing sha256 in local_file_created claim: {claim}"))?;
        verify_sha256_hex("local_file_created sha256", claimed_sha256)?;
        let content_sha256 = claim_value(claim, "content_sha256").ok_or_else(|| {
            format!("Missing content_sha256 in local_file_created claim: {claim}")
        })?;
        verify_sha256_hex("local_file_created content_sha256", content_sha256)?;
        let claimed_byte_length = claim_value(claim, "byte_length")
            .ok_or_else(|| format!("Missing byte_length in local_file_created claim: {claim}"))?
            .parse::<u64>()
            .map_err(|_| "local_file_created byte_length is invalid.".to_string())?;
        let verification_method = claim_value(claim, "verification_method").ok_or_else(|| {
            format!("Missing verification_method in local_file_created claim: {claim}")
        })?;
        if !matches!(
            verification_method,
            "exact_serialized_bytes" | "production_structural_content_verifier"
        ) {
            return Err("local_file_created verification_method is invalid.".to_string());
        }

        let path = claim_path_value(claim)
            .ok_or_else(|| format!("Missing path in local_file_created claim: {claim}"))?;
        let path = PathBuf::from(path);
        if path.starts_with(&self.root) {
            self.guard_claim_path(&path)?;
        } else {
            self.guard_approved_external_write_claim_path(&path)?;
        }

        let metadata = fs::metadata(&path).map_err(|error| {
            format!(
                "Created-file verification could not read {}: {error}",
                path.display()
            )
        })?;
        if !metadata.is_file() {
            return Err(format!(
                "Created-file claim does not point to a file: {}",
                path.display()
            ));
        }
        if metadata.len() != claimed_byte_length {
            return Err(format!(
                "Created-file byte length mismatch for {}.",
                path.display()
            ));
        }

        let actual_format = path
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase)
            .ok_or_else(|| {
                format!(
                    "Created-file claim has no readable file extension: {}",
                    path.display()
                )
            })?;
        if actual_format != format {
            return Err(format!(
                "Created-file format mismatch for {}: claim {}, extension {}.",
                path.display(),
                format,
                actual_format
            ));
        }

        let actual_sha256 = sha256_file_hex(&path).map_err(|error| {
            format!(
                "Created-file verification could not hash {}: {error}",
                path.display()
            )
        })?;
        if actual_sha256 != claimed_sha256 {
            return Err(format!(
                "Created-file digest mismatch for {}.",
                path.display()
            ));
        }

        Ok(())
    }

    fn verify_artifact_claim(&self, claim: &str) -> Result<(), String> {
        if claim_value(claim, "artifact_verified") != Some("true") {
            return Err("Artifact evidence must assert artifact_verified=true.".to_string());
        }
        let claimed_sha256 = claim_value(claim, "sha256")
            .ok_or_else(|| "Artifact evidence is missing sha256.".to_string())?;
        verify_sha256_hex("artifact sha256", claimed_sha256)?;
        let claimed_byte_length = claim_value(claim, "byte_length")
            .ok_or_else(|| "Artifact evidence is missing byte_length.".to_string())?
            .parse::<u64>()
            .map_err(|_| "Artifact evidence byte_length is invalid.".to_string())?;
        if claimed_byte_length == 0 {
            return Err("Artifact evidence byte_length must be greater than zero.".to_string());
        }
        let path = claim_path_value(claim)
            .ok_or_else(|| "Artifact evidence is missing path.".to_string())?;
        let expected = format!(
            "artifact_verified=true path={path} sha256={claimed_sha256} byte_length={claimed_byte_length}"
        );
        if claim != expected {
            return Err(
                "Artifact evidence contains fields outside its verified contract.".to_string(),
            );
        }
        let path = PathBuf::from(path);
        if path.starts_with(&self.root) {
            self.guard_claim_path(&path)?;
        } else {
            self.guard_approved_external_write_claim_path(&path)?;
        }
        let metadata = fs::metadata(&path).map_err(|error| {
            format!(
                "Artifact verification could not read {}: {error}",
                path.display()
            )
        })?;
        if !metadata.is_file() {
            return Err(format!(
                "Artifact claim does not point to a regular file: {}",
                path.display()
            ));
        }
        if metadata.len() != claimed_byte_length {
            return Err(format!(
                "Artifact byte length mismatch for {}.",
                path.display()
            ));
        }
        let actual_sha256 = sha256_file_hex(&path).map_err(|error| {
            format!(
                "Artifact verification could not hash {}: {error}",
                path.display()
            )
        })?;
        if actual_sha256 != claimed_sha256 {
            return Err(format!("Artifact digest mismatch for {}.", path.display()));
        }
        Ok(())
    }

    fn verify_directory_entries_claim(&self, claim: &str) -> Result<(), String> {
        let count = claim_value(claim, "count")
            .ok_or_else(|| format!("Missing count in claim: {claim}"))?
            .parse::<usize>()
            .map_err(|error| format!("Invalid directory_entries count in claim: {error}"))?;
        if count > 1_000_000 {
            return Err(format!(
                "directory_entries count is implausibly large: {count}"
            ));
        }

        if let Some(path) = claim_path_value(claim) {
            let path = PathBuf::from(path);
            self.guard_claim_path(&path)?;
            let metadata = fs::metadata(&path).map_err(|error| {
                format!(
                    "Directory entries claim path failed for {}: {error}",
                    path.display()
                )
            })?;
            if !metadata.is_dir() {
                return Err(format!(
                    "Directory entries claim path is not a directory: {}",
                    path.display()
                ));
            }
        }

        Ok(())
    }

    fn verify_approved_external_write_claim(&self, claim: &str) -> Result<(), String> {
        let path =
            claim_path_value(claim).ok_or_else(|| format!("Missing path in claim: {claim}"))?;
        let min_bytes = claim_value(claim, "min_bytes")
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        let path = PathBuf::from(path);
        self.guard_approved_external_write_claim_path(&path)?;
        let metadata = fs::metadata(&path).map_err(|error| {
            format!(
                "Approved external write claim failed for {}: {error}",
                path.display()
            )
        })?;
        if !metadata.is_file() {
            return Err(format!(
                "Approved external write target is not a file: {}",
                path.display()
            ));
        }
        if metadata.len() < min_bytes {
            return Err(format!(
                "Approved external write target {} has {} bytes, expected at least {min_bytes}.",
                path.display(),
                metadata.len()
            ));
        }

        Ok(())
    }

    fn verify_local_certificate_claim(
        &self,
        claim: &str,
        identity: &SovereignIdentity,
    ) -> Result<(), String> {
        let certificate_hash = claim_value(claim, "local_certificate_hash")
            .ok_or_else(|| format!("Missing local_certificate_hash in claim: {claim}"))?;
        verify_sha256_hex("local_certificate_hash", certificate_hash)?;
        let output_sha256 = claim_value(claim, "output_sha256")
            .ok_or_else(|| format!("Missing output_sha256 in claim: {claim}"))?;
        verify_sha256_hex("output_sha256", output_sha256)?;
        let certificate_b64 = claim_value(claim, "local_certificate_b64")
            .ok_or_else(|| format!("Missing local_certificate_b64 in claim: {claim}"))?;

        let certificate_json = BASE64_STANDARD
            .decode(certificate_b64)
            .map_err(|error| format!("Invalid local_certificate_b64 in claim: {error}"))?;
        let expected_certificate_hash = sha256_hex(&certificate_json);
        if certificate_hash != expected_certificate_hash {
            return Err(
                "local_certificate_hash does not match decoded local_certificate_b64 payload."
                    .to_string(),
            );
        }

        let certificate: LogicalCertificate = serde_json::from_slice(&certificate_json)
            .map_err(|error| format!("Invalid local certificate JSON in claim: {error}"))?;
        if certificate
            .premises
            .iter()
            .any(|premise| premise.trim().is_empty())
            || certificate.premises.is_empty()
        {
            return Err("Local certificate premises must contain non-empty entries.".to_string());
        }
        if certificate
            .execution_path
            .iter()
            .any(|entry| entry.trim().is_empty())
            || certificate.execution_path.is_empty()
        {
            return Err(
                "Local certificate execution_path must contain non-empty entries.".to_string(),
            );
        }
        if certificate.formal_conclusion.trim().is_empty() {
            return Err("Local certificate formal_conclusion is required.".to_string());
        }
        let output_premise = format!("output_sha256={output_sha256}");
        if !certificate
            .premises
            .iter()
            .any(|premise| premise.trim() == output_premise)
        {
            return Err(
                "Local certificate does not bind the claimed output_sha256 premise.".to_string(),
            );
        }
        let signature = certificate
            .signature
            .as_ref()
            .ok_or_else(|| "Local certificate is missing a signature.".to_string())?;
        identity
            .verify_certificate_parts(
                &certificate.premises,
                &certificate.execution_path,
                &certificate.formal_conclusion,
                signature,
            )
            .map_err(|error| {
                format!(
                    "Local certificate signature verification failed: {}",
                    error.message
                )
            })?;

        Ok(())
    }

    fn verify_state_resumed_claim(
        &self,
        claim: &str,
        identity: &SovereignIdentity,
    ) -> Result<(), String> {
        let node_id = claim_value(claim, "node_id")
            .ok_or_else(|| format!("Missing node_id in claim: {claim}"))?;
        let expected_sequence_id = claim_value(claim, "expected_sequence_id")
            .or_else(|| claim_value(claim, "sequence_id"))
            .ok_or_else(|| format!("Missing expected_sequence_id in claim: {claim}"))?
            .parse::<i64>()
            .map_err(|error| format!("Invalid expected_sequence_id in claim: {error}"))?;
        let mission_id = claim_value(claim, "mission_id")
            .ok_or_else(|| format!("Missing mission_id in claim: {claim}"))?;
        let run_id = claim_value(claim, "run_id");
        let architect_signature = parse_signature_claim_value(claim, "mlc_signature_json")?;
        let node_signature = parse_signature_claim_value(claim, "node_signature_json")?;
        let payload =
            state_resumed_claim_payload(node_id, mission_id, run_id, expected_sequence_id);
        identity
            .verify_payload(&payload, &architect_signature)
            .map_err(|error| {
                format!(
                    "state_resumed architect signature invalid: {}",
                    error.message
                )
            })?;
        identity
            .verify_node_payload(&payload, &node_signature)
            .map_err(|error| format!("state_resumed node signature invalid: {}", error.message))?;

        let audit_sequence_id =
            self.audit_expected_sequence_id(node_id, mission_id, run_id.as_deref())?;
        if audit_sequence_id != expected_sequence_id {
            return Err(format!(
                "state_resumed claim contradicts audit_024.sqlite: claimed sequence {}, ledger sequence {} for node {} mission {}.",
                expected_sequence_id, audit_sequence_id, node_id, mission_id
            ));
        }

        Ok(())
    }

    fn audit_expected_sequence_id(
        &self,
        node_id: &str,
        mission_id: &str,
        run_id: Option<&str>,
    ) -> Result<i64, String> {
        let connection = self.open_audit_connection()?;
        let sql = if run_id.is_some() {
            "
            SELECT COALESCE(MAX(sequence_id), 0)
            FROM pre_alpha_mission_chronicle
            WHERE node_id=?1 AND mission_id=?2 AND run_id=?3
            "
        } else {
            "
            SELECT COALESCE(MAX(sequence_id), 0)
            FROM pre_alpha_mission_chronicle
            WHERE node_id=?1 AND mission_id=?2
            "
        };
        let sequence_id = if let Some(run_id) = run_id {
            connection
                .query_row(sql, params![node_id, mission_id, run_id], |row| {
                    row.get::<_, i64>(0)
                })
                .map_err(|error| format!("Failed to query audit_024.sqlite: {error}"))?
        } else {
            connection
                .query_row(sql, params![node_id, mission_id], |row| {
                    row.get::<_, i64>(0)
                })
                .map_err(|error| format!("Failed to query audit_024.sqlite: {error}"))?
        };
        if sequence_id == 0 {
            return Err(format!(
                "state_resumed claim has no matching audit_024.sqlite ledger entry for node {} mission {}.",
                node_id, mission_id
            ));
        }
        Ok(sequence_id)
    }

    fn open_audit_connection(&self) -> Result<Connection, String> {
        let path = self.root.join(AUDIT_DB_PATH);
        self.guard_absolute_path(&path)?;
        Connection::open(path).map_err(|error| format!("Failed to open audit_024.sqlite: {error}"))
    }

    fn guard_absolute_path(&self, path: &Path) -> Result<(), String> {
        if path.is_absolute() && path.starts_with(&self.root) {
            return Ok(());
        }

        Err(format!(
            "Verifier rejected non-quarantined claim path: {}",
            path.display()
        ))
    }

    fn guard_claim_path(&self, path: &Path) -> Result<(), String> {
        if path.is_absolute() && path.starts_with(&self.root) {
            return Ok(());
        }

        Err(format!(
            "Verifier rejected non-quarantined claim path: {}",
            path.display()
        ))
    }

    fn guard_approved_external_write_claim_path(&self, path: &Path) -> Result<(), String> {
        if !path.is_absolute() {
            return Err(format!(
                "Approved external write claim requires an absolute path: {}",
                path.display()
            ));
        }
        if path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        {
            return Err(format!(
                "Approved external write claim rejected path traversal: {}",
                path.display()
            ));
        }
        if path.starts_with(&self.root) {
            return Err(format!(
                "Approved external write claim unexpectedly targeted project quarantine: {}",
                path.display()
            ));
        }

        Ok(())
    }

    fn write_verifier_log(
        &self,
        mlc_path: &str,
        claims: &[String],
        failures: &[String],
    ) -> Result<String, VerificationError> {
        let log_dir = self.root.join("logs").join("verifier");
        fs::create_dir_all(&log_dir).map_err(|error| VerificationError {
            message: format!("Failed to create verifier log directory: {error}"),
            log_path: None,
        })?;

        let path = log_dir.join(format!("verify-{}.md", unix_time_ms()));
        let status = if failures.is_empty() {
            "verified"
        } else {
            "invalid"
        };
        let body = format!(
            "# MLC Verification\n\n- MLC: {mlc_path}\n- Status: {status}\n- Claims Checked: {}\n\n## Claims\n{}\n\n## Failures\n{}\n",
            claims.len(),
            list_lines(claims),
            list_lines(failures),
        );

        fs::write(&path, body).map_err(|error| VerificationError {
            message: format!("Failed to write verifier log: {error}"),
            log_path: None,
        })?;

        Ok(path.to_string_lossy().to_string())
    }

    fn write_plan_verifier_log(
        &self,
        plan: &ActionPlan,
        failures: &[String],
    ) -> Result<String, VerificationError> {
        let log_dir = self.root.join("logs").join("verifier");
        fs::create_dir_all(&log_dir).map_err(|error| VerificationError {
            message: format!("Failed to create verifier log directory: {error}"),
            log_path: None,
        })?;

        let path = log_dir.join(format!("preflight-{}-{}.md", plan.id, unix_time_ms()));
        let body = format!(
            "# ActionPlan Pre-flight Verification\n\n- Plan ID: {}\n- Objective: {}\n- Status: invalid\n- Steps Checked: {}\n\n## Failures\n{}\n",
            plan.id,
            plan.objective,
            plan.steps.len(),
            list_lines(failures),
        );

        fs::write(&path, body).map_err(|error| VerificationError {
            message: format!("Failed to write plan verifier log: {error}"),
            log_path: None,
        })?;

        Ok(path.to_string_lossy().to_string())
    }
}

fn expected_plan_premises(plan: &ActionPlan) -> Vec<String> {
    vec![
        format!("objective={}", plan.objective),
        format!("plan_id={}", plan.id),
    ]
}

fn expected_plan_execution_path(plan: &ActionPlan) -> Vec<String> {
    plan.steps
        .iter()
        .enumerate()
        .map(|(index, step)| {
            format!(
                "{}. step={} tool={} risk={:?}",
                index + 1,
                step.step,
                step.tool.authorization_kind(),
                step.risk_level
            )
        })
        .collect()
}

fn preflight_failure_message(failures: &[String]) -> String {
    let summary = failures
        .iter()
        .map(|failure| failure.trim())
        .filter(|failure| !failure.is_empty())
        .take(3)
        .collect::<Vec<_>>()
        .join("; ");
    if summary.is_empty() {
        return format!(
            "Pre-flight ActionPlan verification failed for {} issue(s).",
            failures.len()
        );
    }

    format!(
        "Pre-flight ActionPlan verification failed for {} issue(s): {}",
        failures.len(),
        summary
    )
}

fn extract_claims(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| line.trim().strip_prefix("- CLAIM "))
        .map(ToString::to_string)
        .collect()
}

fn state_resumed_claim_payload(
    node_id: &str,
    mission_id: &str,
    run_id: Option<&str>,
    expected_sequence_id: i64,
) -> String {
    serde_json::json!({
        "claim": "state_resumed",
        "node_id": node_id,
        "mission_id": mission_id,
        "run_id": run_id,
        "expected_sequence_id": expected_sequence_id
    })
    .to_string()
}

fn parse_signature_claim_value(claim: &str, key: &str) -> Result<SignatureBlock, String> {
    let raw = claim_value(claim, key).ok_or_else(|| format!("Missing {key} in claim: {claim}"))?;
    serde_json::from_str(raw).map_err(|error| format!("Invalid {key} JSON in claim: {error}"))
}

fn claim_value<'a>(claim: &'a str, key: &str) -> Option<&'a str> {
    claim.split_whitespace().find_map(|part| {
        let (candidate, value) = part.split_once('=')?;
        if candidate == key {
            Some(value)
        } else {
            None
        }
    })
}

fn claim_path_value(claim: &str) -> Option<String> {
    let value = claim.split_once("path=")?.1;
    let end = [
        " min_bytes=",
        " count=",
        " status=",
        " node_id=",
        " hash=",
        " sha256=",
        " byte_length=",
        " signature_json=",
        " semantic_pass=",
        " relevance_score=",
        " reasoning_b64=",
        " reasoning_hash=",
    ]
    .iter()
    .filter_map(|marker| value.find(marker))
    .min()
    .unwrap_or(value.len());
    let path = value[..end].trim();
    (!path.is_empty()).then(|| path.to_string())
}

fn verify_sha256_hex(label: &str, value: &str) -> Result<(), String> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Ok(());
    }

    Err(format!(
        "{label} must be a 64-character lowercase hex digest."
    ))
}

fn verify_semantic_reasoning_claim(operation: &str, claim: &str) -> Result<(), String> {
    let semantic_pass = claim_value(claim, "semantic_pass");
    if !is_semantic_operation(operation) && semantic_pass.is_none() {
        return Ok(());
    }
    if semantic_pass != Some("true") {
        return Err(format!(
            "Semantic operation {operation} did not assert semantic_pass=true."
        ));
    }

    let relevance_score = claim_value(claim, "relevance_score")
        .ok_or_else(|| format!("Semantic operation {operation} is missing relevance_score."))?
        .parse::<f64>()
        .map_err(|error| {
            format!("Semantic operation {operation} has an invalid relevance_score: {error}")
        })?;
    if !relevance_score.is_finite() || !(0.0..=1.0).contains(&relevance_score) {
        return Err(format!(
            "Semantic operation {operation} relevance_score must be finite and between 0 and 1."
        ));
    }
    if relevance_score <= 0.0 {
        return Err(format!(
            "Semantic operation {operation} cannot claim a semantic pass with a zero relevance_score."
        ));
    }

    let reasoning_b64 = claim_value(claim, "reasoning_b64")
        .ok_or_else(|| format!("Semantic operation {operation} is missing reasoning_b64."))?;
    let reasoning = BASE64_STANDARD
        .decode(reasoning_b64)
        .map_err(|error| {
            format!("Semantic operation {operation} reasoning_b64 is invalid: {error}")
        })
        .and_then(|bytes| {
            String::from_utf8(bytes).map_err(|error| {
                format!("Semantic operation {operation} reasoning is not UTF-8: {error}")
            })
        })?;
    let reasoning = reasoning.trim();
    if reasoning.len() < 24 {
        return Err(format!(
            "Semantic operation {operation} reasoning block is empty or too short."
        ));
    }
    for required in ["score=", "factors=", "decision="] {
        if !reasoning.contains(required) {
            return Err(format!(
                "Semantic operation {operation} reasoning block is missing required marker '{required}'."
            ));
        }
    }

    let trace_score = reasoning_value(reasoning, "score")
        .and_then(|value| value.parse::<f64>().ok())
        .ok_or_else(|| {
            format!("Semantic operation {operation} reasoning has no parseable score.")
        })?;
    if (trace_score - relevance_score).abs() > 0.0001 {
        return Err(format!(
            "Semantic operation {operation} reasoning score {trace_score:.4} does not match claimed relevance_score {relevance_score:.4}."
        ));
    }

    let reasoning_hash = claim_value(claim, "reasoning_hash")
        .ok_or_else(|| format!("Semantic operation {operation} is missing reasoning_hash."))?;
    let expected_hash = sha256_hex(reasoning.as_bytes());
    if reasoning_hash != expected_hash {
        return Err(format!(
            "Semantic operation {operation} reasoning_hash does not match the decoded reasoning block."
        ));
    }

    Ok(())
}

fn reasoning_value<'a>(reasoning: &'a str, key: &str) -> Option<&'a str> {
    let start = reasoning.find(&format!("{key}="))? + key.len() + 1;
    let value = &reasoning[start..];
    let end = value
        .find(|character: char| character == ';' || character.is_whitespace())
        .unwrap_or(value.len());
    Some(&value[..end])
}

fn is_semantic_operation(operation: &str) -> bool {
    matches!(
        operation,
        "web_fetch"
            | "document_index"
            | "ask_local_document_index"
            | "sovereign_duckduckgo_search"
            | "model_route"
    )
}

fn list_lines(items: &[String]) -> String {
    if items.is_empty() {
        return "- none".to_string();
    }

    items
        .iter()
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn project_root() -> PathBuf {
    crate::settings::app_data_root()
}

#[cfg(test)]
mod tests;
