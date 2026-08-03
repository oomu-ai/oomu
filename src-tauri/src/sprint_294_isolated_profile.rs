//! Release-capable, non-interactive storage isolation for Sprint 294 qualification.
//!
//! Activation is deliberately narrow: the caller must opt in, bind the process
//! to one 17-digit functional run identifier, and provide the exact private
//! application-data directory owned by that run. The process secret and its
//! domain-separated children never leave memory.

use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};
use zeroize::Zeroizing;

pub(crate) const ENABLE_ENV: &str = "OOMU_SPRINT_294_ISOLATED_PROFILE";
pub(crate) const RUN_ID_ENV: &str = "OOMU_SPRINT_294_FUNCTIONAL_RUN_ID";
const RUN_ID_DIGITS: usize = 17;
const RUN_ROOT_PREFIX: &str = "oomu-sprint-294-functional-";
const RESTART_SECRET_ENV: &str = "OOMU_SPRINT_300_RESTART_SECRET";
const DATABASE_SECRET_DOMAIN: &[u8] = b"oomu.sprint-294.isolated.database.v1";
const IDENTITY_SECRET_DOMAIN: &[u8] = b"oomu.sprint-294.isolated.identity.v1";
const QUALIFICATION_KEYCHAIN_GRANT_DOMAIN: &[u8] =
    b"oomu.sprint-302.qualification-keychain-grant.v1";
const QUALIFICATION_KEYCHAIN_GRANT_FILE: &str = ".oomu-sprint-302-keychain-grant.json";

struct ActiveProfile {
    app_data_root: PathBuf,
    run_id: String,
    process_secret: Zeroizing<[u8; 32]>,
}

static ACTIVE_PROFILE: OnceLock<ActiveProfile> = OnceLock::new();

#[derive(Clone, Debug)]
struct PathSecurityFacts {
    canonical_path: PathBuf,
    is_directory: bool,
    is_symlink: bool,
    unix_mode: u32,
    owned_by_process_user: bool,
}

#[derive(Clone, Debug)]
struct RootSecurityFacts {
    run_parent: PathSecurityFacts,
    app_data: PathSecurityFacts,
}

pub(crate) fn activate(app_data_root_env: &str, root: Option<&Path>) -> Result<(), String> {
    if !activation_requested(std::env::var_os(ENABLE_ENV).as_deref()) {
        return Ok(());
    }
    let run_id = std::env::var(RUN_ID_ENV)
        .map_err(|_| format!("{ENABLE_ENV}=1 requires a 17-digit {RUN_ID_ENV}."))?;
    let root = root.ok_or_else(|| {
        format!("{ENABLE_ENV}=1 requires {app_data_root_env} to name its private app-data root.")
    })?;
    let facts = inspect_root(root)?;
    validate_activation_contract(&run_id, root, &facts)?;

    let restart_secret = std::env::var(RESTART_SECRET_ENV).ok();
    let secret = restart_secret
        .as_deref()
        .map(parse_restart_secret)
        .transpose()?
        .unwrap_or_else(|| {
            let mut generated = Zeroizing::new([0_u8; 32]);
            OsRng.fill_bytes(generated.as_mut());
            generated
        });
    ACTIVE_PROFILE
        .set(ActiveProfile {
            app_data_root: root.to_path_buf(),
            run_id: run_id.clone(),
            process_secret: secret,
        })
        .map_err(|_| "Sprint 294 isolated profile was already initialized.".to_string())?;
    let keychain_mode = if root.join(QUALIFICATION_KEYCHAIN_GRANT_FILE).exists() {
        "qualification_grant_required"
    } else {
        "disabled"
    };
    eprintln!(
        "{}",
        activation_marker(&run_id, root, restart_secret.is_some(), keychain_mode)
    );
    Ok(())
}

pub(crate) fn is_active() -> bool {
    ACTIVE_PROFILE.get().is_some()
}

pub(crate) fn requested() -> bool {
    activation_requested(std::env::var_os(ENABLE_ENV).as_deref())
}

pub(crate) fn app_data_root() -> Option<PathBuf> {
    ACTIVE_PROFILE
        .get()
        .map(|profile| profile.app_data_root.clone())
}

pub(crate) fn run_id() -> Option<String> {
    ACTIVE_PROFILE.get().map(|profile| profile.run_id.clone())
}

pub(crate) fn qualification_keychain_grant_key() -> Option<Zeroizing<[u8; 32]>> {
    ACTIVE_PROFILE
        .get()
        .map(|profile| derive_key(&profile.process_secret, QUALIFICATION_KEYCHAIN_GRANT_DOMAIN))
}

pub(crate) fn database_secret() -> Option<Zeroizing<String>> {
    ACTIVE_PROFILE
        .get()
        .map(|profile| derive_secret(&profile.process_secret, DATABASE_SECRET_DOMAIN))
}

pub(crate) fn identity_passphrase() -> Option<Zeroizing<String>> {
    ACTIVE_PROFILE.get().map(|profile| {
        let secret = derive_secret(&profile.process_secret, IDENTITY_SECRET_DOMAIN);
        Zeroizing::new(format!(
            "OOMU Sprint 294 isolated identity {}",
            secret.as_str()
        ))
    })
}

pub(crate) fn knowledge_vault_root() -> Option<PathBuf> {
    app_data_root().map(|root| root.join(".oomu").join("vault"))
}

fn activation_requested(value: Option<&OsStr>) -> bool {
    value == Some(OsStr::new("1"))
}

fn activation_marker(run_id: &str, root: &Path, restartable: bool, keychain_mode: &str) -> String {
    let key_material = if restartable {
        "restartable_process_memory"
    } else {
        "process_memory"
    };
    format!(
        "OOMU_SPRINT_294_ISOLATED_PROFILE status=enabled run_id={run_id} storage=isolated_encrypted key_material={key_material} keychain={keychain_mode} app_data={}",
        root.display()
    )
}

fn parse_restart_secret(value: &str) -> Result<Zeroizing<[u8; 32]>, String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "{RESTART_SECRET_ENV} must contain exactly 64 hexadecimal characters."
        ));
    }
    let bytes =
        hex::decode(value).map_err(|_| format!("{RESTART_SECRET_ENV} could not be decoded."))?;
    let mut secret = Zeroizing::new([0_u8; 32]);
    secret.copy_from_slice(&bytes);
    Ok(secret)
}

fn derive_secret(process_secret: &[u8; 32], domain: &[u8]) -> Zeroizing<String> {
    Zeroizing::new(hex::encode(derive_key(process_secret, domain).as_ref()))
}

fn derive_key(process_secret: &[u8; 32], domain: &[u8]) -> Zeroizing<[u8; 32]> {
    let mut digest = Sha256::new();
    digest.update(domain);
    digest.update([0]);
    digest.update(process_secret);
    let mut key = Zeroizing::new([0_u8; 32]);
    key.copy_from_slice(&digest.finalize());
    key
}

fn inspect_root(root: &Path) -> Result<RootSecurityFacts, String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = root;
        return Err("Sprint 294 isolated profile is supported only on macOS.".to_string());
    }
    #[cfg(target_os = "macos")]
    {
        let parent = root
            .parent()
            .ok_or_else(|| "Sprint 294 isolated app-data root has no run parent.".to_string())?;
        Ok(RootSecurityFacts {
            run_parent: inspect_path(parent, "run parent")?,
            app_data: inspect_path(root, "app-data root")?,
        })
    }
}

#[cfg(target_os = "macos")]
fn inspect_path(path: &Path, label: &str) -> Result<PathSecurityFacts, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Sprint 294 isolated {label} could not be inspected: {error}"))?;
    let canonical_path = fs::canonicalize(path).map_err(|error| {
        format!("Sprint 294 isolated {label} could not be canonicalized: {error}")
    })?;
    // SAFETY: `geteuid` has no pointer arguments or caller-side preconditions.
    let process_uid = unsafe { libc::geteuid() };
    Ok(PathSecurityFacts {
        canonical_path,
        is_directory: metadata.is_dir(),
        is_symlink: metadata.file_type().is_symlink(),
        unix_mode: metadata.mode() & 0o7777,
        owned_by_process_user: metadata.uid() == process_uid,
    })
}

fn validate_activation_contract(
    run_id: &str,
    root: &Path,
    facts: &RootSecurityFacts,
) -> Result<(), String> {
    if run_id.len() != RUN_ID_DIGITS || !run_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!(
            "{RUN_ID_ENV} must contain exactly 17 ASCII digits."
        ));
    }
    let expected = PathBuf::from(format!("/private/tmp/{RUN_ROOT_PREFIX}{run_id}/app-data"));
    let expected_parent = expected
        .parent()
        .expect("fixed Sprint 294 app-data path has a parent");
    if root != expected
        || facts.app_data.canonical_path != expected
        || facts.run_parent.canonical_path != expected_parent
    {
        return Err(format!(
            "Sprint 294 isolated app-data root must be exactly {}.",
            expected.display()
        ));
    }
    if facts.app_data.is_symlink
        || !facts.app_data.is_directory
        || facts.run_parent.is_symlink
        || !facts.run_parent.is_directory
    {
        return Err(
            "Sprint 294 isolated app-data root must be an existing real directory.".to_string(),
        );
    }
    if facts.app_data.unix_mode != 0o700
        || !facts.app_data.owned_by_process_user
        || facts.run_parent.unix_mode != 0o700
        || !facts.run_parent.owned_by_process_user
    {
        return Err(
            "Sprint 294 isolated app-data root must be process-owned with mode 0700.".to_string(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUN_ID: &str = "17850000000000001";

    fn expected_root() -> PathBuf {
        PathBuf::from(format!("/private/tmp/{RUN_ROOT_PREFIX}{RUN_ID}/app-data"))
    }

    fn secure_path(path: PathBuf) -> PathSecurityFacts {
        PathSecurityFacts {
            canonical_path: path,
            is_directory: true,
            is_symlink: false,
            unix_mode: 0o700,
            owned_by_process_user: true,
        }
    }

    fn secure_facts() -> RootSecurityFacts {
        let root = expected_root();
        RootSecurityFacts {
            run_parent: secure_path(root.parent().unwrap().to_path_buf()),
            app_data: secure_path(root),
        }
    }

    #[test]
    fn activation_contract_accepts_only_exact_private_run_root() {
        assert!(validate_activation_contract(RUN_ID, &expected_root(), &secure_facts()).is_ok());

        for invalid_id in [
            "1785000000000001",
            "178500000000000001",
            "1785000000000000x",
        ] {
            assert!(
                validate_activation_contract(invalid_id, &expected_root(), &secure_facts())
                    .is_err()
            );
        }
        assert!(validate_activation_contract(
            RUN_ID,
            Path::new("/private/tmp/other/app-data"),
            &secure_facts(),
        )
        .is_err());
    }

    #[test]
    fn activation_contract_rejects_links_permissions_and_ownership_drift() {
        let mut facts = secure_facts();
        facts.app_data.is_symlink = true;
        assert!(validate_activation_contract(RUN_ID, &expected_root(), &facts).is_err());
        facts = secure_facts();
        facts.app_data.unix_mode = 0o755;
        assert!(validate_activation_contract(RUN_ID, &expected_root(), &facts).is_err());
        facts = secure_facts();
        facts.run_parent.owned_by_process_user = false;
        assert!(validate_activation_contract(RUN_ID, &expected_root(), &facts).is_err());
        facts = secure_facts();
        facts.run_parent.canonical_path = PathBuf::from("/private/tmp/redirected");
        assert!(validate_activation_contract(RUN_ID, &expected_root(), &facts).is_err());
        facts = secure_facts();
        facts.run_parent.unix_mode = 0o755;
        assert!(validate_activation_contract(RUN_ID, &expected_root(), &facts).is_err());
    }

    #[test]
    fn activation_flag_and_secret_domains_are_exact() {
        assert!(activation_requested(Some(OsStr::new("1"))));
        assert!(!activation_requested(Some(OsStr::new("true"))));
        assert!(!activation_requested(None));

        let process_secret = [7_u8; 32];
        let database = derive_secret(&process_secret, DATABASE_SECRET_DOMAIN);
        let identity = derive_secret(&process_secret, IDENTITY_SECRET_DOMAIN);
        let grant = derive_key(&process_secret, QUALIFICATION_KEYCHAIN_GRANT_DOMAIN);
        assert_ne!(database.as_str(), identity.as_str());
        assert_ne!(database.as_bytes(), grant.as_ref());
        assert_ne!(identity.as_bytes(), grant.as_ref());
        assert_eq!(database.len(), 64);
        assert_eq!(identity.len(), 64);
        assert_eq!(grant.len(), 32);
        assert_eq!(
            activation_marker(RUN_ID, &expected_root(), false, "disabled"),
            format!(
                "OOMU_SPRINT_294_ISOLATED_PROFILE status=enabled run_id={RUN_ID} storage=isolated_encrypted key_material=process_memory keychain=disabled app_data={}",
                expected_root().display()
            )
        );
        assert!(activation_marker(
            RUN_ID,
            &expected_root(),
            true,
            "qualification_grant_required",
        )
        .contains("keychain=qualification_grant_required"));
        assert_eq!(
            parse_restart_secret(&"07".repeat(32)).unwrap().as_ref(),
            &[7_u8; 32]
        );
        assert!(parse_restart_secret("not-a-secret").is_err());
    }
}
