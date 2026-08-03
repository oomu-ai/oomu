use crate::{macos_process_identity, runtime_profile};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) const RECEIPT_KIND: &str = "single_instance";
pub(crate) const RECEIPT_SCHEMA_VERSION: u8 = 2;
pub(crate) const HOLDER_SCHEMA_VERSION: u8 = 3;
pub(crate) const ACTIVATION_PROTOCOL_VERSION: u8 = 2;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstanceIdentity {
    pub(crate) release_channel: String,
    pub(crate) profile_class: String,
    pub(crate) bundle_identifier: Option<String>,
    pub(crate) build_number: u64,
    pub(crate) code_directory_hash: Option<String>,
    pub(crate) executable_sha256: Option<String>,
    pub(crate) strict_signature_valid: bool,
    pub(crate) namespace: String,
}

impl InstanceIdentity {
    pub(crate) fn from_process(
        process: &macos_process_identity::MacosProcessIdentityEvidence,
        profile_class: runtime_profile::RuntimeProfileClass,
    ) -> Self {
        let mut identity = Self {
            release_channel: process.release_channel.to_string(),
            profile_class: profile_class.as_str().to_string(),
            bundle_identifier: process.bundle_identifier.clone(),
            build_number: process.build_number,
            code_directory_hash: process.code_directory_hash.clone(),
            executable_sha256: process.executable_sha256.clone(),
            strict_signature_valid: process.strict_signature_valid,
            namespace: String::new(),
        };
        identity.namespace = namespace_for(&identity);
        identity
    }

    pub(crate) fn matches_holder(&self, holder: &Self) -> bool {
        self == holder && !self.namespace.is_empty()
    }
}

fn namespace_for(identity: &InstanceIdentity) -> String {
    let stable = serde_json::json!({
        "releaseChannel": identity.release_channel,
        "profileClass": identity.profile_class,
        "bundleIdentifier": identity.bundle_identifier,
        "buildNumber": identity.build_number,
        "codeDirectoryHash": identity.code_directory_hash,
        "executableSha256": identity.executable_sha256,
        "strictSignatureValid": identity.strict_signature_valid,
    });
    let digest = Sha256::digest(
        serde_json::to_vec(&stable).expect("single-instance identity is JSON serializable"),
    );
    format!("{:x}", digest)[..24].to_string()
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub(crate) enum HolderRecord {
    Verifying {
        schema_version: u8,
        pid: u32,
    },
    Ready {
        schema_version: u8,
        pid: u32,
        identity: InstanceIdentity,
    },
}

impl HolderRecord {
    pub(crate) fn verifying() -> Self {
        Self::Verifying {
            schema_version: HOLDER_SCHEMA_VERSION,
            pid: std::process::id(),
        }
    }

    pub(crate) fn current(identity: InstanceIdentity) -> Self {
        Self::Ready {
            schema_version: HOLDER_SCHEMA_VERSION,
            pid: std::process::id(),
            identity,
        }
    }

    pub(crate) fn is_supported(&self) -> bool {
        match self {
            Self::Verifying {
                schema_version,
                pid,
            }
            | Self::Ready {
                schema_version,
                pid,
                ..
            } => *schema_version == HOLDER_SCHEMA_VERSION && *pid > 0,
        }
    }

    pub(crate) fn ready(&self) -> Option<(u32, &InstanceIdentity)> {
        match self {
            Self::Ready { pid, identity, .. } if self.is_supported() => Some((*pid, identity)),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActivationMessage {
    pub(crate) protocol_version: u8,
    pub(crate) namespace: String,
}

impl ActivationMessage {
    pub(crate) fn for_identity(identity: &InstanceIdentity) -> Self {
        Self {
            protocol_version: ACTIVATION_PROTOCOL_VERSION,
            namespace: identity.namespace.clone(),
        }
    }

    pub(crate) fn accepts(&self, identity: &InstanceIdentity) -> bool {
        self.protocol_version == ACTIVATION_PROTOCOL_VERSION && self.namespace == identity.namespace
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SingleInstanceReceipt<'a> {
    kind: &'static str,
    schema_version: u8,
    decision: &'a str,
    namespace: &'a str,
    release_channel: &'a str,
    profile_class: &'a str,
    build_number: u64,
    code_directory_hash: Option<&'a str>,
    strict_signature_valid: bool,
    holder_pid: Option<u32>,
}

impl<'a> SingleInstanceReceipt<'a> {
    pub(crate) fn new(
        decision: &'a str,
        identity: &'a InstanceIdentity,
        holder_pid: Option<u32>,
    ) -> Self {
        Self {
            kind: RECEIPT_KIND,
            schema_version: RECEIPT_SCHEMA_VERSION,
            decision,
            namespace: &identity.namespace,
            release_channel: &identity.release_channel,
            profile_class: &identity.profile_class,
            build_number: identity.build_number,
            code_directory_hash: identity.code_directory_hash.as_deref(),
            strict_signature_valid: identity.strict_signature_valid,
            holder_pid,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(build_number: u64, profile_class: &str) -> InstanceIdentity {
        let mut identity = InstanceIdentity {
            release_channel: "development".to_string(),
            profile_class: profile_class.to_string(),
            bundle_identifier: Some("ai.eldris.oomu.gpd.development".to_string()),
            build_number,
            code_directory_hash: Some("abcdef".to_string()),
            executable_sha256: Some("123456".to_string()),
            strict_signature_valid: true,
            namespace: String::new(),
        };
        identity.namespace = namespace_for(&identity);
        identity
    }

    #[test]
    fn build_and_profile_are_part_of_the_instance_namespace() {
        let first = identity(1, "development");
        let next_build = identity(2, "development");
        let qualification = identity(1, "qualification");

        assert_ne!(first.namespace, next_build.namespace);
        assert_ne!(first.namespace, qualification.namespace);
        assert!(!first.matches_holder(&next_build));
        assert!(!first.matches_holder(&qualification));
    }

    #[test]
    fn mismatched_instance_identity_is_not_activated() {
        let first = identity(1, "development");
        let other = identity(2, "development");
        let message = ActivationMessage::for_identity(&first);

        assert!(!first.matches_holder(&other));
        assert!(message.accepts(&first));
        assert!(!message.accepts(&other));
    }

    #[test]
    fn verifying_holder_is_supported_but_never_ready_for_activation() {
        let holder = HolderRecord::verifying();

        assert!(holder.is_supported());
        assert!(holder.ready().is_none());
        let encoded = serde_json::to_string(&holder).unwrap();
        assert!(encoded.contains("\"state\":\"verifying\""));
        assert_eq!(
            serde_json::from_str::<HolderRecord>(&encoded).unwrap(),
            holder
        );
    }

    #[test]
    fn ready_holder_binds_the_complete_instance_identity() {
        let expected = identity(4, "development");
        let holder = HolderRecord::current(expected.clone());

        assert_eq!(holder.ready(), Some((std::process::id(), &expected)));
        let encoded = serde_json::to_string(&holder).unwrap();
        assert!(encoded.contains("\"state\":\"ready\""));
        assert_eq!(
            serde_json::from_str::<HolderRecord>(&encoded).unwrap(),
            holder
        );
    }
}
