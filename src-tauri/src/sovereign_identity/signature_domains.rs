use super::{IdentityError, SignatureBlock, SovereignIdentity};
use serde::{Deserialize, Serialize};

const AUTHORITY_CERTIFICATE_DOMAIN: &str = "oomu.authority-certificate.v2";
const NATIVE_FILE_AUTHORITY_DOMAIN: &str = "oomu.native-file-authority.v1";
const NATIVE_EVIDENCE_DOMAIN: &str = "oomu.native-evidence.v2";
pub(crate) const NATIVE_FILE_AUTHORITY_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeFileAuthorityClaim {
    pub version: u32,
    pub nonce: String,
    pub action_kind: String,
    pub canonical_target: String,
    pub canonical_root: String,
    pub session_id: String,
    pub turn_id: String,
    pub generation_token: String,
    pub agent_id: String,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct NativeFileAuthorityEnvelope {
    pub claim: NativeFileAuthorityClaim,
    pub signature: SignatureBlock,
}

fn domain_payload(domain: &str, payload: &str) -> String {
    serde_json::json!({
        "domain": domain,
        "payload": payload,
    })
    .to_string()
}

pub(crate) fn authority_certificate_payload(
    premises: &[String],
    execution_path: &[String],
    formal_conclusion: &str,
) -> String {
    domain_payload(
        AUTHORITY_CERTIFICATE_DOMAIN,
        &legacy_certificate_payload(premises, execution_path, formal_conclusion),
    )
}

pub(crate) fn legacy_certificate_payload(
    premises: &[String],
    execution_path: &[String],
    formal_conclusion: &str,
) -> String {
    serde_json::json!({
        "premises": premises,
        "execution_path": execution_path,
        "formal_conclusion": formal_conclusion,
    })
    .to_string()
}

pub(crate) fn native_evidence_payload(payload: &str) -> String {
    domain_payload(NATIVE_EVIDENCE_DOMAIN, payload)
}

fn native_file_authority_payload(
    claim: &NativeFileAuthorityClaim,
) -> Result<String, IdentityError> {
    let claim = serde_json::to_string(claim).map_err(|error| {
        IdentityError::invalid(format!("Native authority claim is invalid: {error}"))
    })?;
    Ok(domain_payload(NATIVE_FILE_AUTHORITY_DOMAIN, &claim))
}

impl SovereignIdentity {
    pub fn sign_certificate_parts(
        &self,
        premises: &[String],
        execution_path: &[String],
        formal_conclusion: &str,
    ) -> Result<SignatureBlock, IdentityError> {
        self.sign_exact_payload(&authority_certificate_payload(
            premises,
            execution_path,
            formal_conclusion,
        ))
    }

    pub fn verify_certificate_parts(
        &self,
        premises: &[String],
        execution_path: &[String],
        formal_conclusion: &str,
        signature: &SignatureBlock,
    ) -> Result<(), IdentityError> {
        if self
            .verify_authority_certificate_parts(
                premises,
                execution_path,
                formal_conclusion,
                signature,
            )
            .is_ok()
        {
            return Ok(());
        }
        self.verify_exact_current_payload(
            &legacy_certificate_payload(premises, execution_path, formal_conclusion),
            signature,
        )
    }

    pub fn verify_authority_certificate_parts(
        &self,
        premises: &[String],
        execution_path: &[String],
        formal_conclusion: &str,
        signature: &SignatureBlock,
    ) -> Result<(), IdentityError> {
        self.verify_exact_current_payload(
            &authority_certificate_payload(premises, execution_path, formal_conclusion),
            signature,
        )
    }

    pub(crate) fn sign_native_file_authority(
        &self,
        claim: NativeFileAuthorityClaim,
    ) -> Result<NativeFileAuthorityEnvelope, IdentityError> {
        if claim.version != NATIVE_FILE_AUTHORITY_VERSION {
            return Err(IdentityError::invalid(
                "Native file authority version is not supported.".to_string(),
            ));
        }
        let payload = native_file_authority_payload(&claim)?;
        let signature = self.sign_exact_payload(&payload)?;
        Ok(NativeFileAuthorityEnvelope { claim, signature })
    }

    pub(crate) fn verify_native_file_authority(
        &self,
        envelope: &NativeFileAuthorityEnvelope,
    ) -> Result<(), IdentityError> {
        let payload = native_file_authority_payload(&envelope.claim)?;
        self.verify_exact_current_payload(&payload, &envelope.signature)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> SovereignIdentity {
        SovereignIdentity::initialize_with_session_passphrase(
            "OOMU signature domain separation test identity",
        )
        .expect("identity initializes")
    }

    #[test]
    fn authority_and_evidence_envelopes_are_not_interchangeable() {
        let identity = identity();
        let premises = vec!["observed=true".to_string()];
        let path = vec!["perform one bounded operation".to_string()];
        let conclusion = "authorized once";
        let authority = identity
            .sign_certificate_parts(&premises, &path, conclusion)
            .expect("authority signs");
        let legacy_payload = legacy_certificate_payload(&premises, &path, conclusion);

        assert!(identity
            .verify_payload(&legacy_payload, &authority)
            .is_err());
        let evidence = identity
            .sign_payload(&legacy_payload)
            .expect("evidence signs");
        assert!(identity
            .verify_authority_certificate_parts(&premises, &path, conclusion, &evidence)
            .is_err());
    }

    #[test]
    fn evidence_signatures_preserve_the_raw_payload_digest_contract() {
        let identity = identity();
        let payload = r#"{"artifact":"verified"}"#;
        let evidence = identity.sign_payload(payload).expect("evidence signs");

        assert_eq!(
            evidence.payload_hash,
            super::super::sha256_hex(payload.as_bytes())
        );
        assert_ne!(
            evidence.payload_hash,
            super::super::sha256_hex(native_evidence_payload(payload).as_bytes()),
        );
        identity
            .verify_payload(payload, &evidence)
            .expect("domain-separated evidence verifies against raw content");
    }

    #[test]
    fn native_file_authority_is_not_accepted_as_evidence_or_certificate() {
        let identity = identity();
        let claim = NativeFileAuthorityClaim {
            version: NATIVE_FILE_AUTHORITY_VERSION,
            nonce: "native-file-nonce".to_string(),
            action_kind: "file_read".to_string(),
            canonical_target: "/tmp/document.txt".to_string(),
            canonical_root: "/tmp".to_string(),
            session_id: "session-a".to_string(),
            turn_id: "turn-a".to_string(),
            generation_token: "generation-a".to_string(),
            agent_id: "agent-a".to_string(),
            issued_at_ms: 10,
            expires_at_ms: 20,
        };
        let authority = identity
            .sign_native_file_authority(claim.clone())
            .expect("native authority signs");
        let claim_json = serde_json::to_string(&claim).expect("claim serializes");
        assert!(identity
            .verify_payload(&claim_json, &authority.signature)
            .is_err());
        assert!(identity
            .verify_authority_certificate_parts(
                &["observed=true".to_string()],
                &["read one file".to_string()],
                "authorized once",
                &authority.signature,
            )
            .is_err());

        let evidence = identity
            .sign_payload(&claim_json)
            .expect("evidence signs in its own domain");
        assert!(identity
            .verify_native_file_authority(&NativeFileAuthorityEnvelope {
                claim,
                signature: evidence,
            })
            .is_err());
    }

    #[test]
    fn legacy_certificate_is_history_only() {
        let identity = identity();
        let premises = vec!["historical=true".to_string()];
        let path = vec!["record completed work".to_string()];
        let conclusion = "historical evidence";
        let payload = legacy_certificate_payload(&premises, &path, conclusion);
        let legacy = identity
            .sign_exact_payload(&payload)
            .expect("legacy fixture signs");

        identity
            .verify_certificate_parts(&premises, &path, conclusion, &legacy)
            .expect("history remains readable");
        assert!(identity
            .verify_authority_certificate_parts(&premises, &path, conclusion, &legacy)
            .is_err());
    }
}
