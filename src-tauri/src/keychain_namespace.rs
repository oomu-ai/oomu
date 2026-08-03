use std::sync::OnceLock;

const PRODUCTION_CREDENTIAL_SERVICE: &str = "ai.eldris.oomu.backend-credentials";
const DEVELOPMENT_CREDENTIAL_SERVICE: &str = "ai.eldris.oomu.development.backend-credentials";
const QUALIFICATION_CREDENTIAL_SERVICE: &str = "ai.eldris.oomu.qualification.backend-credentials";
#[cfg(test)]
const TEST_CREDENTIAL_SERVICE: &str = "ai.eldris.oomu.backend-credentials.test";

const PRODUCTION_IDENTITY_SERVICE: &str = "OOMU Sovereign Identity";
const PRODUCTION_IDENTITY_ACCOUNT: &str = "oomu-ed25519-genesis";
const DEVELOPMENT_IDENTITY_SERVICE: &str = "OOMU Development Sovereign Identity";
const DEVELOPMENT_IDENTITY_ACCOUNT: &str = "oomu-development-ed25519-genesis";
const QUALIFICATION_IDENTITY_SERVICE: &str = "OOMU Qualification Sovereign Identity";
const QUALIFICATION_IDENTITY_ACCOUNT: &str = "oomu-qualification-ed25519-genesis";
#[cfg(test)]
const TEST_IDENTITY_SERVICE: &str = "OOMU Test Sovereign Identity";
#[cfg(test)]
const TEST_IDENTITY_ACCOUNT: &str = "oomu-test-ed25519-genesis";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum KeychainNamespace {
    Production,
    Development,
    Qualification,
    #[cfg(test)]
    Test,
}

#[cfg(not(test))]
static CURRENT_NAMESPACE: OnceLock<KeychainNamespace> = OnceLock::new();

#[cfg(not(test))]
pub(crate) fn bind_verified_process_identity(
    identity: &crate::macos_process_identity::MacosProcessIdentityEvidence,
) -> Result<(), &'static str> {
    bind_namespace(&CURRENT_NAMESPACE, namespace_for_process_identity(identity))
}

#[cfg(test)]
pub(crate) fn bind_verified_process_identity(
    _identity: &crate::macos_process_identity::MacosProcessIdentityEvidence,
) -> Result<(), &'static str> {
    Ok(())
}

fn bind_namespace(
    slot: &OnceLock<KeychainNamespace>,
    namespace: KeychainNamespace,
) -> Result<(), &'static str> {
    if let Some(bound) = slot.get() {
        return (*bound == namespace)
            .then_some(())
            .ok_or("verified process identity conflicts with the active keychain namespace");
    }
    match slot.set(namespace) {
        Ok(()) => Ok(()),
        Err(candidate) if slot.get() == Some(&candidate) => Ok(()),
        Err(_) => Err("verified process identity conflicts with the active keychain namespace"),
    }
}

pub(crate) fn application_data_identifier() -> &'static str {
    match current_namespace() {
        KeychainNamespace::Production => "ai.eldris.oomu.gpd",
        KeychainNamespace::Development => "ai.eldris.oomu.gpd.development",
        KeychainNamespace::Qualification => "ai.eldris.oomu.gpd.qualification",
        #[cfg(test)]
        KeychainNamespace::Test => "ai.eldris.oomu.gpd.test",
    }
}

pub(crate) fn backend_credentials_service() -> &'static str {
    match current_namespace() {
        KeychainNamespace::Production => PRODUCTION_CREDENTIAL_SERVICE,
        KeychainNamespace::Development => DEVELOPMENT_CREDENTIAL_SERVICE,
        KeychainNamespace::Qualification => QUALIFICATION_CREDENTIAL_SERVICE,
        #[cfg(test)]
        KeychainNamespace::Test => TEST_CREDENTIAL_SERVICE,
    }
}

pub(crate) fn sovereign_identity_location() -> (&'static str, &'static str) {
    match current_namespace() {
        KeychainNamespace::Production => (PRODUCTION_IDENTITY_SERVICE, PRODUCTION_IDENTITY_ACCOUNT),
        KeychainNamespace::Development => {
            (DEVELOPMENT_IDENTITY_SERVICE, DEVELOPMENT_IDENTITY_ACCOUNT)
        }
        KeychainNamespace::Qualification => (
            QUALIFICATION_IDENTITY_SERVICE,
            QUALIFICATION_IDENTITY_ACCOUNT,
        ),
        #[cfg(test)]
        KeychainNamespace::Test => (TEST_IDENTITY_SERVICE, TEST_IDENTITY_ACCOUNT),
    }
}

#[cfg(test)]
fn current_namespace() -> KeychainNamespace {
    KeychainNamespace::Test
}

#[cfg(not(test))]
fn current_namespace() -> KeychainNamespace {
    *CURRENT_NAMESPACE
        .get_or_init(|| namespace_for_process_identity(&crate::macos_process_identity::current()))
}

fn namespace_for_process_identity(
    identity: &crate::macos_process_identity::MacosProcessIdentityEvidence,
) -> KeychainNamespace {
    if crate::launch_startup::sprint_294_isolated_profile::is_active() {
        return KeychainNamespace::Qualification;
    }
    if identity.release_channel == "production"
        && identity.bundle_identifier.as_deref() == Some("ai.eldris.oomu.gpd")
        && identity.team_id.as_deref() == Some("R7AQ8287N6")
        && identity
            .signing_authority
            .as_deref()
            .is_some_and(|value| value.starts_with("Developer ID Application:"))
        && identity.hardened_runtime
        && identity.strict_signature_valid
    {
        KeychainNamespace::Production
    } else {
        KeychainNamespace::Development
    }
}

pub(crate) fn namespace_class() -> &'static str {
    match current_namespace() {
        KeychainNamespace::Production => "production",
        KeychainNamespace::Development => "development",
        KeychainNamespace::Qualification => "qualification",
        #[cfg(test)]
        KeychainNamespace::Test => "test",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::macos_process_identity::MacosProcessIdentityEvidence;

    fn identity(
        release_channel: &'static str,
        bundle_identifier: &str,
        team_id: Option<&str>,
        signing_authority: Option<&str>,
        hardened_runtime: bool,
        strict_signature_valid: bool,
    ) -> MacosProcessIdentityEvidence {
        MacosProcessIdentityEvidence {
            requesting_process: "oomu".to_string(),
            release_channel,
            bundle_identifier: Some(bundle_identifier.to_string()),
            team_id: team_id.map(str::to_string),
            signing_authority: signing_authority.map(str::to_string),
            build_number: 1,
            code_directory_hash: Some("abc123".to_string()),
            executable_sha256: Some("def456".to_string()),
            signature_artifact_sha256: strict_signature_valid.then(|| "a".repeat(64)),
            signature_verification_exit_status: Some(if strict_signature_valid { 0 } else { 1 }),
            signature_verification_failure_code: (!strict_signature_valid)
                .then_some("signature_verification_failed"),
            designated_requirement_sha256: None,
            hardened_runtime,
            strict_signature_valid,
        }
    }

    #[test]
    fn only_the_verified_production_identity_uses_production_secrets() {
        assert_eq!(
            namespace_for_process_identity(&identity(
                "production",
                "ai.eldris.oomu.gpd",
                Some("R7AQ8287N6"),
                Some("Developer ID Application: Eldris AI LLC (R7AQ8287N6)"),
                true,
                true,
            )),
            KeychainNamespace::Production
        );
    }

    #[test]
    fn development_and_unverified_builds_never_use_production_secrets() {
        for candidate in [
            identity(
                "development",
                "ai.eldris.oomu.gpd.development",
                None,
                None,
                true,
                true,
            ),
            identity(
                "unidentified",
                "ai.eldris.oomu.gpd",
                Some("R7AQ8287N6"),
                Some("Developer ID Application: Eldris AI LLC (R7AQ8287N6)"),
                true,
                false,
            ),
            identity(
                "production",
                "ai.eldris.oomu.gpd",
                Some("OTHERTEAM1"),
                Some("Developer ID Application: Other (OTHERTEAM1)"),
                true,
                true,
            ),
            identity(
                "production",
                "ai.eldris.oomu.gpd",
                Some("R7AQ8287N6"),
                Some("Apple Development: Developer (R7AQ8287N6)"),
                true,
                true,
            ),
        ] {
            assert_eq!(
                namespace_for_process_identity(&candidate),
                KeychainNamespace::Development
            );
        }
    }

    #[test]
    fn test_builds_use_an_isolated_in_memory_namespace() {
        assert_eq!(backend_credentials_service(), TEST_CREDENTIAL_SERVICE);
        assert_eq!(
            sovereign_identity_location(),
            (TEST_IDENTITY_SERVICE, TEST_IDENTITY_ACCOUNT)
        );
        assert_eq!(application_data_identifier(), "ai.eldris.oomu.gpd.test");
    }

    #[test]
    fn verified_namespace_binding_is_idempotent_and_rejects_conflicts() {
        let slot = OnceLock::new();
        assert_eq!(
            bind_namespace(&slot, KeychainNamespace::Development),
            Ok(())
        );
        assert_eq!(
            bind_namespace(&slot, KeychainNamespace::Development),
            Ok(())
        );
        assert_eq!(
            bind_namespace(&slot, KeychainNamespace::Production),
            Err("verified process identity conflicts with the active keychain namespace")
        );
    }
}
