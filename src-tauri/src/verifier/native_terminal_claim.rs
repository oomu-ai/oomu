use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use std::{collections::BTreeMap, path::PathBuf};

const CLAIM_NAME: &str = "native_terminal_receipt";
const REQUIRED_FIELDS: [&str; 10] = [
    "schema",
    "evidence_kind",
    "request_sha256",
    "command_b64",
    "cwd_b64",
    "env_keys_b64",
    "exit_status",
    "timed_out",
    "postcondition_verified",
    "direct_process",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerifiedNativeTerminalClaim {
    pub command: String,
    pub cwd: PathBuf,
}

pub(super) fn verify(claim: &str) -> Result<(), String> {
    parse_and_verify(claim).map(|_| ())
}

pub(crate) fn parse_and_verify(claim: &str) -> Result<VerifiedNativeTerminalClaim, String> {
    let mut parts = claim.split_whitespace();
    if parts.next() != Some(CLAIM_NAME) {
        return Err("Native terminal receipt has the wrong claim name.".to_string());
    }

    let mut fields = BTreeMap::new();
    for part in parts {
        let (key, value) = part
            .split_once('=')
            .ok_or_else(|| format!("Native terminal receipt field is malformed: {part}"))?;
        if !REQUIRED_FIELDS.contains(&key) {
            return Err(format!(
                "Native terminal receipt has an unknown field: {key}"
            ));
        }
        if fields.insert(key, value).is_some() {
            return Err(format!("Native terminal receipt repeats field: {key}"));
        }
    }
    for field in REQUIRED_FIELDS {
        if !fields.contains_key(field) {
            return Err(format!("Native terminal receipt is missing field: {field}"));
        }
    }

    require_exact(&fields, "schema", "oomu.native_terminal.v1")?;
    require_exact(&fields, "evidence_kind", "observed_native")?;
    require_exact(&fields, "exit_status", "0")?;
    require_exact(&fields, "timed_out", "false")?;
    require_exact(&fields, "postcondition_verified", "true")?;
    require_exact(&fields, "direct_process", "true")?;

    let digest = fields["request_sha256"];
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(
            "Native terminal receipt request_sha256 must be a lowercase SHA-256 digest."
                .to_string(),
        );
    }

    let command = decode_text("command_b64", fields["command_b64"], 32_768)?;
    if command.trim().is_empty() {
        return Err("Native terminal receipt command is empty.".to_string());
    }
    let cwd = decode_text("cwd_b64", fields["cwd_b64"], 4_096)?;
    let cwd = PathBuf::from(cwd);
    if !cwd.is_absolute() || !cwd.is_dir() {
        return Err(
            "Native terminal receipt working folder is not an available absolute directory."
                .to_string(),
        );
    }
    let environment_keys = decode_text("env_keys_b64", fields["env_keys_b64"], 4_096)?;
    if environment_keys
        .split(',')
        .filter(|key| !key.is_empty())
        .any(|key| {
            !key.bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
    {
        return Err("Native terminal receipt environment key list is invalid.".to_string());
    }

    Ok(VerifiedNativeTerminalClaim { command, cwd })
}

fn require_exact(fields: &BTreeMap<&str, &str>, key: &str, expected: &str) -> Result<(), String> {
    if fields.get(key).copied() == Some(expected) {
        return Ok(());
    }
    Err(format!("Native terminal receipt {key} must be {expected}."))
}

fn decode_text(label: &str, encoded: &str, max_bytes: usize) -> Result<String, String> {
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| format!("Native terminal receipt {label} is not valid base64url."))?;
    if bytes.len() > max_bytes || bytes.contains(&0) {
        return Err(format!("Native terminal receipt {label} is invalid."));
    }
    String::from_utf8(bytes).map_err(|_| format!("Native terminal receipt {label} is not UTF-8."))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_claim() -> String {
        let cwd = std::env::temp_dir().canonicalize().unwrap();
        format!(
            "native_terminal_receipt schema=oomu.native_terminal.v1 evidence_kind=observed_native request_sha256={} command_b64={} cwd_b64={} env_keys_b64= exit_status=0 timed_out=false postcondition_verified=true direct_process=true",
            "a".repeat(64),
            URL_SAFE_NO_PAD.encode(b"/usr/bin/git status --short --branch"),
            URL_SAFE_NO_PAD.encode(cwd.to_string_lossy().as_bytes()),
        )
    }

    #[test]
    fn accepts_complete_observed_native_terminal_receipt() {
        verify(&valid_claim()).expect("receipt should verify");
    }

    #[test]
    fn rejects_terminal_receipt_without_a_verified_postcondition() {
        let claim = valid_claim().replace(
            "postcondition_verified=true",
            "postcondition_verified=false",
        );
        assert!(verify(&claim).is_err());
    }

    #[test]
    fn rejects_terminal_receipt_with_duplicate_or_unknown_fields() {
        assert!(verify(&format!("{} timed_out=false", valid_claim())).is_err());
        assert!(verify(&format!("{} untrusted=true", valid_claim())).is_err());
    }
}
