use crate::macos_process_identity::{
    code_identity, production_identity_is_invalid, MacosProcessIdentityEvidence,
};
use serde::Serialize;
use std::fmt;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

const RECEIPT_KIND: &str = "runtime_identity";
const RECEIPT_SCHEMA_VERSION: u8 = 2;
const RECEIPT_RELATIVE_PATH: &str = ".oomu/runtime-profile-receipt.json";

pub(crate) const INVALID_PRODUCTION_IDENTITY: &str = "runtime_profile_invalid_production_identity";
pub(crate) const PRODUCTION_OVERRIDE_REJECTED: &str =
    "runtime_profile_production_override_rejected";
pub(crate) const VALIDATION_REQUIRED: &str = "runtime_profile_validation_required";
pub(crate) const IDENTITY_NOT_AUTHORIZED: &str = "runtime_profile_identity_not_authorized";
pub(crate) const QUALIFICATION_REQUIRED: &str = "runtime_profile_qualification_required";
pub(crate) const IDENTITY_UNRECOGNIZED: &str = "runtime_profile_identity_unrecognized";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeProfileFailure {
    pub(crate) code: &'static str,
    pub(crate) detail: String,
}

impl RuntimeProfileFailure {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for RuntimeProfileFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.detail)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RuntimeProfileClass {
    Production,
    Development,
    Qualification,
}

impl RuntimeProfileClass {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Development => "development",
            Self::Qualification => "qualification",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeProfileReceipt {
    pub(crate) kind: &'static str,
    pub(crate) schema_version: u8,
    pub(crate) status: &'static str,
    pub(crate) channel: &'static str,
    pub(crate) profile_class: RuntimeProfileClass,
    pub(crate) bundle_identifier: Option<String>,
    pub(crate) team_id: Option<String>,
    pub(crate) application_data_namespace: &'static str,
    pub(crate) keychain_namespace_class: &'static str,
    pub(crate) build_number: u64,
    pub(crate) code_directory_hash: Option<String>,
    pub(crate) executable_sha256: Option<String>,
    pub(crate) designated_requirement_sha256: Option<String>,
    pub(crate) strict_signature_valid: bool,
    pub(crate) signature_artifact_sha256: Option<String>,
    pub(crate) signature_verification_exit_status: Option<i32>,
    pub(crate) signature_verification_failure_code: Option<&'static str>,
    pub(crate) release_integrity_status: &'static str,
    pub(crate) single_instance_namespace: String,
}

pub(crate) fn validate_request(
    identity: &MacosProcessIdentityEvidence,
    alternate_root_present: bool,
    qualification_requested: bool,
) -> Result<RuntimeProfileClass, RuntimeProfileFailure> {
    if production_identity_is_invalid(identity) {
        return Err(RuntimeProfileFailure::new(
            INVALID_PRODUCTION_IDENTITY,
            "the production bundle failed strict code-signature identity verification",
        ));
    }
    if identity.release_channel == "production"
        && (alternate_root_present || qualification_requested)
    {
        return Err(RuntimeProfileFailure::new(
            PRODUCTION_OVERRIDE_REJECTED,
            "the production identity requested an alternate or qualification profile",
        ));
    }
    if alternate_root_present && !qualification_requested {
        return Err(RuntimeProfileFailure::new(
            VALIDATION_REQUIRED,
            "an alternate data root was present without a validated qualification profile",
        ));
    }
    if qualification_requested
        && !matches!(identity.release_channel, "development" | "qualification")
    {
        return Err(RuntimeProfileFailure::new(
            IDENTITY_NOT_AUTHORIZED,
            "the current release channel is not authorized for qualification storage",
        ));
    }

    match identity.release_channel {
        "production" => Ok(RuntimeProfileClass::Production),
        "development" if qualification_requested => Ok(RuntimeProfileClass::Qualification),
        "development" => Ok(RuntimeProfileClass::Development),
        "qualification" if qualification_requested => Ok(RuntimeProfileClass::Qualification),
        "qualification" => Err(RuntimeProfileFailure::new(
            QUALIFICATION_REQUIRED,
            "a qualification identity started without its validated qualification profile",
        )),
        _ => Err(RuntimeProfileFailure::new(
            IDENTITY_UNRECOGNIZED,
            "the executable identity does not map to a supported release channel",
        )),
    }
}

pub(crate) fn current_class(
    identity: &MacosProcessIdentityEvidence,
) -> Result<RuntimeProfileClass, RuntimeProfileFailure> {
    let qualification = crate::launch_startup::sprint_294_isolated_profile::is_active();
    validate_request(identity, qualification, qualification)
}

pub(crate) fn receipt(
    identity: &MacosProcessIdentityEvidence,
    instance_namespace: String,
) -> Result<RuntimeProfileReceipt, RuntimeProfileFailure> {
    let profile_class = current_class(identity)?;
    if identity.strict_signature_valid && !identity.strict_signature_is_freshly_bound() {
        return Err(RuntimeProfileFailure::new(
            INVALID_PRODUCTION_IDENTITY,
            "the signature result is not bound to a fresh exact-artifact verification",
        ));
    }
    Ok(RuntimeProfileReceipt {
        kind: RECEIPT_KIND,
        schema_version: RECEIPT_SCHEMA_VERSION,
        status: "verified",
        channel: identity.release_channel,
        profile_class,
        bundle_identifier: identity.bundle_identifier.clone(),
        team_id: identity.team_id.clone(),
        application_data_namespace: crate::keychain_namespace::application_data_identifier(),
        keychain_namespace_class: crate::keychain_namespace::namespace_class(),
        build_number: identity.build_number,
        code_directory_hash: identity.code_directory_hash.clone(),
        executable_sha256: identity.executable_sha256.clone(),
        designated_requirement_sha256: identity.designated_requirement_sha256.clone(),
        strict_signature_valid: identity.strict_signature_valid,
        signature_artifact_sha256: identity.signature_artifact_sha256.clone(),
        signature_verification_exit_status: identity.signature_verification_exit_status,
        signature_verification_failure_code: identity.signature_verification_failure_code,
        release_integrity_status: if profile_class == RuntimeProfileClass::Production {
            "verified"
        } else {
            "not_applicable"
        },
        single_instance_namespace: instance_namespace,
    })
}

pub(crate) fn write_receipt(
    app_data_root: &Path,
    receipt: &RuntimeProfileReceipt,
) -> Result<PathBuf, String> {
    let destination = app_data_root.join(RECEIPT_RELATIVE_PATH);
    let parent = destination
        .parent()
        .ok_or_else(|| "OOMU could not prepare its runtime receipt.".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("OOMU could not prepare its runtime receipt: {error}"))?;
    let temporary = destination.with_extension(format!("json.tmp-{}", std::process::id()));
    let contents = serde_json::to_vec_pretty(receipt)
        .map_err(|error| format!("OOMU could not encode its runtime receipt: {error}"))?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| format!("OOMU could not prepare its runtime receipt: {error}"))?;
    file.write_all(&contents)
        .and_then(|_| file.write_all(b"\n"))
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("OOMU could not save its runtime receipt: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("OOMU could not secure its runtime receipt: {error}"))?;
    }
    fs::rename(&temporary, &destination)
        .map_err(|error| format!("OOMU could not publish its runtime receipt: {error}"))?;
    Ok(destination)
}

pub(crate) fn identity_component(identity: &MacosProcessIdentityEvidence) -> &str {
    code_identity(identity).unwrap_or("unavailable")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(
        channel: &'static str,
        bundle_identifier: &str,
        signature_valid: bool,
    ) -> MacosProcessIdentityEvidence {
        MacosProcessIdentityEvidence {
            requesting_process: "oomu".to_string(),
            release_channel: channel,
            bundle_identifier: Some(bundle_identifier.to_string()),
            team_id: None,
            signing_authority: None,
            build_number: 2,
            code_directory_hash: Some("abc123".to_string()),
            executable_sha256: Some("def456".to_string()),
            signature_artifact_sha256: signature_valid.then(|| "c".repeat(64)),
            signature_verification_exit_status: Some(if signature_valid { 0 } else { 1 }),
            signature_verification_failure_code: (!signature_valid)
                .then_some("signature_verification_failed"),
            designated_requirement_sha256: None,
            hardened_runtime: signature_valid,
            strict_signature_valid: signature_valid,
        }
    }

    #[test]
    fn production_rejects_unvalidated_test_profile_override() {
        let production = identity("production", "ai.eldris.oomu.gpd", true);
        assert!(validate_request(&production, true, true).is_err());
        assert!(validate_request(&production, true, false).is_err());
        assert_eq!(
            validate_request(&production, false, false),
            Ok(RuntimeProfileClass::Production)
        );
    }

    #[test]
    fn invalid_production_identity_stops_instead_of_using_development_storage() {
        let invalid = identity("unidentified", "ai.eldris.oomu.gpd", false);
        let error = validate_request(&invalid, false, false).unwrap_err();
        assert_eq!(error.code, INVALID_PRODUCTION_IDENTITY);
    }

    #[test]
    fn alternate_root_requires_validated_qualification_profile() {
        let development = identity("development", "ai.eldris.oomu.gpd.development", true);
        assert!(validate_request(&development, true, false).is_err());
        assert_eq!(
            validate_request(&development, true, true),
            Ok(RuntimeProfileClass::Qualification)
        );
    }

    #[test]
    fn sprint_304_development_receipt_never_claims_release_integrity() {
        let development = identity("development", "ai.eldris.oomu.gpd.development", false);
        let receipt = super::receipt(&development, "dev-instance".to_string()).unwrap();
        assert_eq!(receipt.channel, "development");
        assert_eq!(receipt.release_integrity_status, "not_applicable");
        assert!(!receipt.strict_signature_valid);
        assert_eq!(
            receipt.signature_verification_failure_code,
            Some("signature_verification_failed")
        );
    }

    #[test]
    fn sprint_304_receipt_rejects_an_unbound_positive_signature_claim() {
        let mut production = identity("production", "ai.eldris.oomu.gpd", true);
        production.signature_artifact_sha256 = None;
        let error = super::receipt(&production, "prod-instance".to_string()).unwrap_err();
        assert_eq!(error.code, INVALID_PRODUCTION_IDENTITY);
    }
}
