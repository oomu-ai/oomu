use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

pub(super) const GRANT_FILE_NAME: &str = ".oomu-sprint-302-keychain-grant.json";
const GRANT_SCHEMA: &str = "oomu.sprint-302.qualification-keychain-grant.v1";
const QUALIFICATION_SERVICE: &str = "ai.eldris.oomu.qualification.backend-credentials";
const DEVELOPMENT_BUNDLE_ID: &str = "ai.eldris.oomu.gpd.development";
const MAX_GRANT_BYTES: u64 = 8 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Operation {
    Get,
    Exists,
    Retry,
    Set,
    Delete,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct QualificationGrant {
    schema: String,
    run_id: String,
    profile_root_sha256: String,
    service: String,
    account: String,
    provider_config_id: String,
    cloud_contract_sha256: String,
    process_class: String,
    secret_material_included: bool,
    authorization_hmac_sha256: String,
}

struct AuthorizationContext<'a> {
    app_data_root: &'a Path,
    run_id: &'a str,
    service: &'a str,
    account: &'a str,
    release_channel: &'a str,
    requesting_process: &'a str,
    bundle_identifier: Option<&'a str>,
    grant_key: &'a [u8; 32],
}

pub(super) fn authorize(service: &str, account: &str, operation: Operation) -> Result<(), String> {
    if !operation_allowed(operation) {
        return Err(denied());
    }
    let profile =
        crate::launch_startup::sprint_294_isolated_profile::app_data_root().ok_or_else(denied)?;
    let run_id = crate::launch_startup::sprint_294_isolated_profile::run_id().ok_or_else(denied)?;
    let grant_key =
        crate::launch_startup::sprint_294_isolated_profile::qualification_keychain_grant_key()
            .ok_or_else(denied)?;
    let identity = crate::macos_process_identity::current();
    let grant = read_private_grant(&profile)?;
    validate_grant(
        &grant,
        &AuthorizationContext {
            app_data_root: &profile,
            run_id: &run_id,
            service,
            account,
            release_channel: identity.release_channel,
            requesting_process: &identity.requesting_process,
            bundle_identifier: identity.bundle_identifier.as_deref(),
            grant_key: &grant_key,
        },
    )
}

fn operation_allowed(operation: Operation) -> bool {
    matches!(operation, Operation::Get | Operation::Exists)
}

fn denied() -> String {
    "keychain_disabled_in_isolated_qualification_profile".to_string()
}

fn read_private_grant(profile: &Path) -> Result<QualificationGrant, String> {
    let path = profile.join(GRANT_FILE_NAME);
    let metadata = fs::symlink_metadata(&path).map_err(|_| denied())?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() == 0
        || metadata.len() > MAX_GRANT_BYTES
    {
        return Err(denied());
    }
    #[cfg(unix)]
    {
        // SAFETY: `geteuid` has no pointer arguments or caller-side preconditions.
        let current_uid = unsafe { libc::geteuid() };
        if metadata.mode() & 0o777 != 0o600 || metadata.uid() != current_uid {
            return Err(denied());
        }
    }
    let bytes = fs::read(path).map_err(|_| denied())?;
    serde_json::from_slice(&bytes).map_err(|_| denied())
}

fn validate_grant(
    grant: &QualificationGrant,
    context: &AuthorizationContext<'_>,
) -> Result<(), String> {
    let expected_account = format!(
        "provider-{:x}",
        Sha256::digest(grant.provider_config_id.as_bytes())
    );
    let profile_root_sha256 = format!(
        "{:x}",
        Sha256::digest(context.app_data_root.as_os_str().as_encoded_bytes())
    );
    let identity_is_development = context.release_channel == "development"
        && context.requesting_process == "oomu"
        && match context.bundle_identifier {
            Some(identifier) => identifier == DEVELOPMENT_BUNDLE_ID,
            None => cfg!(debug_assertions),
        };
    if grant.schema != GRANT_SCHEMA
        || grant.run_id != context.run_id
        || grant.profile_root_sha256 != profile_root_sha256
        || grant.service != QUALIFICATION_SERVICE
        || grant.service != context.service
        || grant.account != expected_account
        || grant.account != context.account
        || grant.process_class != "development_qualification"
        || grant.secret_material_included
        || !valid_identifier(&grant.provider_config_id)
        || !valid_sha256(&grant.cloud_contract_sha256)
        || !valid_sha256(&grant.authorization_hmac_sha256)
        || !identity_is_development
    {
        return Err(denied());
    }
    let expected_hmac = hmac_sha256(context.grant_key, canonical_payload(grant).as_bytes());
    let provided_hmac = hex::decode(&grant.authorization_hmac_sha256).map_err(|_| denied())?;
    if !constant_time_equal(&expected_hmac, &provided_hmac) {
        return Err(denied());
    }
    Ok(())
}

fn canonical_payload(grant: &QualificationGrant) -> String {
    [
        grant.schema.as_str(),
        grant.run_id.as_str(),
        grant.profile_root_sha256.as_str(),
        grant.service.as_str(),
        grant.account.as_str(),
        grant.provider_config_id.as_str(),
        grant.cloud_contract_sha256.as_str(),
        grant.process_class.as_str(),
        if grant.secret_material_included {
            "true"
        } else {
            "false"
        },
    ]
    .join("\0")
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn hmac_sha256(key: &[u8; 32], message: &[u8]) -> [u8; 32] {
    let mut inner_pad = [0x36_u8; 64];
    let mut outer_pad = [0x5c_u8; 64];
    for (index, byte) in key.iter().enumerate() {
        inner_pad[index] ^= byte;
        outer_pad[index] ^= byte;
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(message);
    let inner_digest = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner_digest);
    outer.finalize().into()
}

fn constant_time_equal(expected: &[u8], provided: &[u8]) -> bool {
    if expected.len() != provided.len() {
        return false;
    }
    expected
        .iter()
        .zip(provided)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[cfg(unix)]
    use std::os::unix::fs::{symlink, PermissionsExt};

    fn grant(root: &Path, run_id: &str, key: &[u8; 32]) -> QualificationGrant {
        let provider_config_id = "sprint-302-real-gemini".to_string();
        let mut value = QualificationGrant {
            schema: GRANT_SCHEMA.to_string(),
            run_id: run_id.to_string(),
            profile_root_sha256: format!(
                "{:x}",
                Sha256::digest(root.as_os_str().as_encoded_bytes())
            ),
            service: QUALIFICATION_SERVICE.to_string(),
            account: format!(
                "provider-{:x}",
                Sha256::digest(provider_config_id.as_bytes())
            ),
            provider_config_id,
            cloud_contract_sha256: "a".repeat(64),
            process_class: "development_qualification".to_string(),
            secret_material_included: false,
            authorization_hmac_sha256: String::new(),
        };
        value.authorization_hmac_sha256 =
            hex::encode(hmac_sha256(key, canonical_payload(&value).as_bytes()));
        value
    }

    fn context<'a>(
        root: &'a Path,
        run_id: &'a str,
        grant: &'a QualificationGrant,
        key: &'a [u8; 32],
    ) -> AuthorizationContext<'a> {
        AuthorizationContext {
            app_data_root: root,
            run_id,
            service: &grant.service,
            account: &grant.account,
            release_channel: "development",
            requesting_process: "oomu",
            bundle_identifier: None,
            grant_key: key,
        }
    }

    #[test]
    fn exact_grant_authorizes_only_read_and_existence_operations() {
        let root =
            PathBuf::from("/private/tmp/oomu-sprint-294-functional-17850000000000001/app-data");
        let key: [u8; 32] =
            hex::decode("6b03a341467407c8dc993820f30011722c598eb4216f87610ad7f43e1d6492f1")
                .unwrap()
                .try_into()
                .unwrap();
        let value = grant(&root, "17850000000000001", &key);
        let context = context(&root, "17850000000000001", &value, &key);
        assert!(validate_grant(&value, &context).is_ok());
        assert!(validate_grant(&value, &context).is_ok());
        assert!(operation_allowed(Operation::Get));
        assert!(operation_allowed(Operation::Exists));
        assert!(!operation_allowed(Operation::Retry));
        assert!(!operation_allowed(Operation::Set));
        assert!(!operation_allowed(Operation::Delete));
        assert_eq!(
            value.authorization_hmac_sha256,
            "62b849d8653f90632397a6f74b3955bf0c81218dba29f40bebc0f30826277db3"
        );
    }

    #[test]
    fn service_account_profile_run_and_identity_mismatches_fail_closed() {
        let root =
            PathBuf::from("/private/tmp/oomu-sprint-294-functional-17850000000000001/app-data");
        let other_root =
            PathBuf::from("/private/tmp/oomu-sprint-294-functional-17850000000000002/app-data");
        let key = [7_u8; 32];
        let value = grant(&root, "17850000000000001", &key);

        let mut wrong_service = context(&root, "17850000000000001", &value, &key);
        wrong_service.service = "ai.eldris.oomu.backend-credentials";
        assert!(validate_grant(&value, &wrong_service).is_err());
        let mut wrong_account = context(&root, "17850000000000001", &value, &key);
        wrong_account.account = "provider-deadbeef";
        assert!(validate_grant(&value, &wrong_account).is_err());
        assert!(validate_grant(
            &value,
            &context(&other_root, "17850000000000001", &value, &key),
        )
        .is_err());
        assert!(
            validate_grant(&value, &context(&root, "17850000000000002", &value, &key),).is_err()
        );

        let mut wrong_identity = context(&root, "17850000000000001", &value, &key);
        wrong_identity.release_channel = "production";
        assert!(validate_grant(&value, &wrong_identity).is_err());
        wrong_identity.release_channel = "development";
        wrong_identity.requesting_process = "other";
        assert!(validate_grant(&value, &wrong_identity).is_err());
        wrong_identity.requesting_process = "oomu";
        wrong_identity.bundle_identifier = Some("ai.eldris.oomu.gpd");
        assert!(validate_grant(&value, &wrong_identity).is_err());
    }

    #[test]
    fn tampering_wrong_key_and_secret_material_fail_closed() {
        let root =
            PathBuf::from("/private/tmp/oomu-sprint-294-functional-17850000000000001/app-data");
        let key = [7_u8; 32];
        let mut value = grant(&root, "17850000000000001", &key);
        value.cloud_contract_sha256 = "b".repeat(64);
        assert!(
            validate_grant(&value, &context(&root, "17850000000000001", &value, &key),).is_err()
        );
        value = grant(&root, "17850000000000001", &key);
        assert!(validate_grant(
            &value,
            &context(&root, "17850000000000001", &value, &[8_u8; 32]),
        )
        .is_err());
        value.secret_material_included = true;
        assert!(
            validate_grant(&value, &context(&root, "17850000000000001", &value, &key),).is_err()
        );
    }

    fn temporary_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "oomu-s302-grant-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        root
    }

    fn serialized_grant(root: &Path) -> Vec<u8> {
        serde_json::to_vec(&grant(root, "17850000000000001", &[7_u8; 32])).unwrap()
    }

    #[test]
    #[cfg(unix)]
    fn private_grant_reader_rejects_mode_link_size_and_malformed_content() {
        let root = temporary_root("file-contract");
        let path = root.join(GRANT_FILE_NAME);
        fs::write(&path, serialized_grant(&root)).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(read_private_grant(&root).is_ok());

        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_private_grant(&root).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(&path, b"not-json").unwrap();
        assert!(read_private_grant(&root).is_err());
        fs::write(&path, vec![b'x'; MAX_GRANT_BYTES as usize + 1]).unwrap();
        assert!(read_private_grant(&root).is_err());

        fs::remove_file(&path).unwrap();
        let target = root.join("grant-target.json");
        fs::write(&target, serialized_grant(&root)).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
        symlink(&target, &path).unwrap();
        assert!(read_private_grant(&root).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
