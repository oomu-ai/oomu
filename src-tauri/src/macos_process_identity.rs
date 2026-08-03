use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{fs, path::Path, process::Command};

const PRODUCTION_IDENTIFIER: &str = "ai.eldris.oomu.gpd";
const DEVELOPMENT_IDENTIFIER: &str = "ai.eldris.oomu.gpd.development";
const QUALIFICATION_IDENTIFIER: &str = "ai.eldris.oomu.gpd.qualification";
const PRODUCTION_TEAM_ID: &str = "R7AQ8287N6";
const RELEASE_BUILD_NUMBER: &str = env!("OOMU_RELEASE_BUILD_NUMBER");

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MacosProcessIdentityEvidence {
    pub requesting_process: String,
    pub release_channel: &'static str,
    pub bundle_identifier: Option<String>,
    pub team_id: Option<String>,
    pub signing_authority: Option<String>,
    pub build_number: u64,
    pub code_directory_hash: Option<String>,
    pub executable_sha256: Option<String>,
    pub signature_artifact_sha256: Option<String>,
    pub signature_verification_exit_status: Option<i32>,
    pub signature_verification_failure_code: Option<&'static str>,
    pub designated_requirement_sha256: Option<String>,
    pub hardened_runtime: bool,
    pub strict_signature_valid: bool,
}

#[cfg(target_os = "macos")]
pub(crate) fn current() -> MacosProcessIdentityEvidence {
    compute_current()
}

#[cfg(target_os = "macos")]
fn compute_current() -> MacosProcessIdentityEvidence {
    let executable = std::env::current_exe().ok();
    let requesting_process = executable
        .as_deref()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or("oomu")
        .to_string();
    let application_bundle = executable.as_deref().and_then(application_bundle_path);
    let detail = application_bundle
        .as_deref()
        .and_then(read_codesign_detail)
        .unwrap_or_default();
    let parsed = parse_codesign_detail(&detail);
    let signature = verify_application_signature(application_bundle.as_deref());
    let strict_signature_valid = signature.is_freshly_bound();
    let release_channel = if cfg!(debug_assertions) && application_bundle.is_none() {
        "development"
    } else {
        release_channel(
            parsed.bundle_identifier.as_deref(),
            parsed.team_id.as_deref(),
            parsed.signing_authority.as_deref(),
            parsed.hardened_runtime,
            strict_signature_valid,
        )
    };
    MacosProcessIdentityEvidence {
        requesting_process,
        release_channel,
        bundle_identifier: parsed.bundle_identifier,
        team_id: parsed.team_id,
        signing_authority: parsed.signing_authority,
        build_number: application_bundle
            .as_deref()
            .and_then(read_bundle_build_number)
            .unwrap_or_else(|| RELEASE_BUILD_NUMBER.parse().unwrap_or_default()),
        code_directory_hash: parsed.code_directory_hash,
        executable_sha256: executable
            .as_deref()
            .and_then(|path| fs::read(path).ok())
            .map(|bytes| format!("{:x}", Sha256::digest(bytes))),
        signature_artifact_sha256: signature.artifact_sha256,
        signature_verification_exit_status: signature.exit_status,
        signature_verification_failure_code: signature.failure_code,
        designated_requirement_sha256: parsed
            .designated_requirement
            .map(|value| format!("{:x}", Sha256::digest(value.as_bytes()))),
        hardened_runtime: parsed.hardened_runtime,
        strict_signature_valid,
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn current() -> MacosProcessIdentityEvidence {
    compute_current()
}

#[cfg(not(target_os = "macos"))]
fn compute_current() -> MacosProcessIdentityEvidence {
    MacosProcessIdentityEvidence {
        requesting_process: "oomu".to_string(),
        release_channel: "unsupported",
        bundle_identifier: None,
        team_id: None,
        signing_authority: None,
        build_number: RELEASE_BUILD_NUMBER.parse().unwrap_or_default(),
        code_directory_hash: None,
        executable_sha256: None,
        signature_artifact_sha256: None,
        signature_verification_exit_status: None,
        signature_verification_failure_code: Some("signature_platform_unsupported"),
        designated_requirement_sha256: None,
        hardened_runtime: false,
        strict_signature_valid: false,
    }
}

impl MacosProcessIdentityEvidence {
    pub(crate) fn strict_signature_is_freshly_bound(&self) -> bool {
        self.strict_signature_valid
            && self.signature_verification_exit_status == Some(0)
            && self.signature_verification_failure_code.is_none()
            && self
                .signature_artifact_sha256
                .as_deref()
                .is_some_and(valid_sha256)
    }
}

#[derive(Debug, Default)]
struct SignatureVerification {
    artifact_sha256: Option<String>,
    exit_status: Option<i32>,
    failure_code: Option<&'static str>,
}

impl SignatureVerification {
    fn is_freshly_bound(&self) -> bool {
        self.exit_status == Some(0)
            && self.failure_code.is_none()
            && self.artifact_sha256.as_deref().is_some_and(valid_sha256)
    }
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[derive(Debug, Default, Eq, PartialEq)]
struct ParsedCodeSignature {
    bundle_identifier: Option<String>,
    team_id: Option<String>,
    signing_authority: Option<String>,
    designated_requirement: Option<String>,
    code_directory_hash: Option<String>,
    hardened_runtime: bool,
}

#[cfg(target_os = "macos")]
fn verify_application_signature(application_bundle: Option<&Path>) -> SignatureVerification {
    let Some(application_bundle) = application_bundle else {
        return SignatureVerification {
            failure_code: Some("signature_artifact_unavailable"),
            ..SignatureVerification::default()
        };
    };
    let before = hash_application_tree(application_bundle);
    let output = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict", "--verbose=4"])
        .arg(application_bundle)
        .output();
    let after = hash_application_tree(application_bundle);
    let exit_status = output.as_ref().ok().and_then(|result| result.status.code());
    let failure_code = if before.is_none() || after.is_none() {
        Some("signature_artifact_hash_failed")
    } else if before != after {
        Some("signature_artifact_changed_during_verification")
    } else {
        match output {
            Err(_) => Some("signature_verifier_unavailable"),
            Ok(result) if result.status.success() => None,
            Ok(result) if result.status.code().is_none() => Some("signature_verifier_terminated"),
            Ok(_) => Some("signature_verification_failed"),
        }
    };
    SignatureVerification {
        artifact_sha256: after,
        exit_status,
        failure_code,
    }
}

#[cfg(target_os = "macos")]
fn hash_application_tree(root: &Path) -> Option<String> {
    use std::io::Read;
    use std::os::unix::ffi::OsStrExt;

    fn visit(root: &Path, directory: &Path, digest: &mut Sha256) -> std::io::Result<()> {
        let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let relative = path.strip_prefix(root).map_err(std::io::Error::other)?;
            let path_bytes = relative.as_os_str().as_bytes();
            digest.update(path_bytes.len().to_be_bytes());
            digest.update(path_bytes);
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                digest.update(b"symlink");
                let target = fs::read_link(&path)?;
                let target_bytes = target.as_os_str().as_bytes();
                digest.update(target_bytes.len().to_be_bytes());
                digest.update(target_bytes);
            } else if file_type.is_dir() {
                digest.update(b"directory");
                visit(root, &path, digest)?;
            } else if file_type.is_file() {
                digest.update(b"file");
                let mut file = fs::File::open(path)?;
                let mut buffer = [0_u8; 64 * 1024];
                loop {
                    let count = file.read(&mut buffer)?;
                    if count == 0 {
                        break;
                    }
                    digest.update(&buffer[..count]);
                }
            } else {
                return Err(std::io::Error::other("unsupported bundle entry"));
            }
        }
        Ok(())
    }

    let mut digest = Sha256::new();
    visit(root, root, &mut digest).ok()?;
    Some(format!("{:x}", digest.finalize()))
}

#[cfg(target_os = "macos")]
fn read_codesign_detail(executable: &Path) -> Option<String> {
    let output = Command::new("/usr/bin/codesign")
        .args(["-d", "--verbose=4", "-r-"])
        .arg(executable)
        .output()
        .ok()?;
    Some(format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

#[cfg(target_os = "macos")]
fn read_bundle_build_number(application_bundle: &Path) -> Option<u64> {
    let info_plist = application_bundle.join("Contents/Info.plist");
    let output = Command::new("/usr/bin/plutil")
        .args(["-extract", "CFBundleVersion", "raw", "-o", "-"])
        .arg(info_plist)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().parse().ok())
        .flatten()
}

fn application_bundle_path(executable: &Path) -> Option<&Path> {
    executable.ancestors().find(|candidate| {
        candidate
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
    })
}

pub(crate) fn current_executable_is_bundled_app() -> bool {
    std::env::current_exe()
        .ok()
        .as_deref()
        .and_then(application_bundle_path)
        .is_some()
}

fn parse_codesign_detail(detail: &str) -> ParsedCodeSignature {
    ParsedCodeSignature {
        bundle_identifier: line_value(detail, "Identifier="),
        team_id: line_value(detail, "TeamIdentifier=").filter(|value| value != "not set"),
        signing_authority: line_value(detail, "Authority="),
        designated_requirement: line_value(detail, "designated => "),
        code_directory_hash: line_value(detail, "CDHash=")
            .filter(|value| value.bytes().all(|byte| byte.is_ascii_hexdigit())),
        hardened_runtime: detail.lines().any(|line| {
            line.starts_with("CodeDirectory ")
                && line
                    .split_once("flags=")
                    .is_some_and(|(_, flags)| flags.contains("runtime"))
        }),
    }
}

fn line_value(detail: &str, prefix: &str) -> Option<String> {
    detail
        .lines()
        .find_map(|line| line.strip_prefix(prefix))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn release_channel(
    identifier: Option<&str>,
    team_id: Option<&str>,
    signing_authority: Option<&str>,
    hardened_runtime: bool,
    strict_signature_valid: bool,
) -> &'static str {
    if identifier == Some(DEVELOPMENT_IDENTIFIER) {
        return "development";
    }
    if identifier == Some(QUALIFICATION_IDENTIFIER) && strict_signature_valid {
        return "qualification";
    }
    if identifier == Some(PRODUCTION_IDENTIFIER)
        && team_id == Some(PRODUCTION_TEAM_ID)
        && signing_authority.is_some_and(|value| value.starts_with("Developer ID Application:"))
        && hardened_runtime
        && strict_signature_valid
    {
        return "production";
    }
    "unidentified"
}

pub(crate) fn production_identity_is_invalid(identity: &MacosProcessIdentityEvidence) -> bool {
    identity.bundle_identifier.as_deref() == Some(PRODUCTION_IDENTIFIER)
        && (identity.release_channel != "production"
            || !identity.strict_signature_is_freshly_bound())
}

pub(crate) fn code_identity(identity: &MacosProcessIdentityEvidence) -> Option<&str> {
    identity
        .code_directory_hash
        .as_deref()
        .or(identity.executable_sha256.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_privacy_safe_process_identity() {
        let parsed = parse_codesign_detail(
            "Identifier=ai.eldris.oomu.gpd\nTeamIdentifier=R7AQ8287N6\n\
             Authority=Developer ID Application: Eldris AI LLC (R7AQ8287N6)\n\
             CDHash=1234567890abcdef1234567890abcdef12345678\n\
             CodeDirectory v=20500 size=123 flags=0x10000(runtime) hashes=1 location=embedded\n\
             designated => identifier \"ai.eldris.oomu.gpd\" and anchor apple generic\n",
        );
        assert_eq!(
            parsed.bundle_identifier.as_deref(),
            Some(PRODUCTION_IDENTIFIER)
        );
        assert_eq!(parsed.team_id.as_deref(), Some("R7AQ8287N6"));
        assert!(parsed.designated_requirement.is_some());
        assert_eq!(
            parsed.code_directory_hash.as_deref(),
            Some("1234567890abcdef1234567890abcdef12345678")
        );
        assert_eq!(
            release_channel(
                parsed.bundle_identifier.as_deref(),
                parsed.team_id.as_deref(),
                parsed.signing_authority.as_deref(),
                parsed.hardened_runtime,
                true,
            ),
            "production"
        );
    }

    #[test]
    fn development_identity_never_reports_production_channel() {
        assert_eq!(
            release_channel(Some(DEVELOPMENT_IDENTIFIER), None, None, false, false),
            "development"
        );
        assert_eq!(
            release_channel(Some("oomu"), None, None, false, false),
            "unidentified"
        );
    }

    #[test]
    fn production_channel_requires_the_reviewed_team_and_a_valid_signature() {
        assert_eq!(
            release_channel(
                Some(PRODUCTION_IDENTIFIER),
                Some(PRODUCTION_TEAM_ID),
                Some("Developer ID Application: Eldris AI LLC (R7AQ8287N6)"),
                true,
                false,
            ),
            "unidentified"
        );
        assert_eq!(
            release_channel(
                Some(PRODUCTION_IDENTIFIER),
                Some("OTHERTEAM1"),
                Some("Developer ID Application: Other (OTHERTEAM1)"),
                true,
                true,
            ),
            "unidentified"
        );
        assert_eq!(
            release_channel(
                Some(PRODUCTION_IDENTIFIER),
                Some(PRODUCTION_TEAM_ID),
                Some("Apple Development: Developer (R7AQ8287N6)"),
                true,
                true,
            ),
            "unidentified"
        );
    }

    #[test]
    fn production_named_invalid_identity_is_a_startup_stop() {
        let identity = MacosProcessIdentityEvidence {
            requesting_process: "oomu".to_string(),
            release_channel: "unidentified",
            bundle_identifier: Some(PRODUCTION_IDENTIFIER.to_string()),
            team_id: Some(PRODUCTION_TEAM_ID.to_string()),
            signing_authority: Some(
                "Developer ID Application: Eldris AI LLC (R7AQ8287N6)".to_string(),
            ),
            build_number: 2,
            code_directory_hash: Some("abc123".to_string()),
            executable_sha256: Some("def456".to_string()),
            signature_artifact_sha256: None,
            signature_verification_exit_status: Some(1),
            signature_verification_failure_code: Some("signature_verification_failed"),
            designated_requirement_sha256: None,
            hardened_runtime: true,
            strict_signature_valid: false,
        };

        assert!(production_identity_is_invalid(&identity));
        assert_eq!(code_identity(&identity), Some("abc123"));
    }

    #[test]
    fn process_identity_requires_a_real_application_bundle() {
        assert_eq!(
            application_bundle_path(Path::new("/Applications/OOMU.app/Contents/MacOS/oomu")),
            Some(Path::new("/Applications/OOMU.app"))
        );
        assert_eq!(
            application_bundle_path(Path::new("/Users/tester/OOMU/target/debug/oomu")),
            None
        );
    }

    #[test]
    fn sprint_304_strict_signature_truth_requires_an_exact_fresh_artifact_binding() {
        let mut identity = MacosProcessIdentityEvidence {
            requesting_process: "oomu".to_string(),
            release_channel: "production",
            bundle_identifier: Some(PRODUCTION_IDENTIFIER.to_string()),
            team_id: Some(PRODUCTION_TEAM_ID.to_string()),
            signing_authority: Some(
                "Developer ID Application: Eldris AI LLC (R7AQ8287N6)".to_string(),
            ),
            build_number: 7,
            code_directory_hash: Some("a".repeat(40)),
            executable_sha256: Some("b".repeat(64)),
            signature_artifact_sha256: Some("c".repeat(64)),
            signature_verification_exit_status: Some(0),
            signature_verification_failure_code: None,
            designated_requirement_sha256: None,
            hardened_runtime: true,
            strict_signature_valid: true,
        };

        assert!(identity.strict_signature_is_freshly_bound());
        assert!(!production_identity_is_invalid(&identity));
        identity.signature_verification_exit_status = Some(1);
        assert!(!identity.strict_signature_is_freshly_bound());
        assert!(production_identity_is_invalid(&identity));
    }

    #[test]
    fn sprint_304_signature_failure_keeps_a_stable_redacted_native_code() {
        let verification = SignatureVerification {
            artifact_sha256: Some("d".repeat(64)),
            exit_status: Some(1),
            failure_code: Some("signature_verification_failed"),
        };
        assert!(!verification.is_freshly_bound());
        assert_eq!(verification.exit_status, Some(1));
        assert_eq!(
            verification.failure_code,
            Some("signature_verification_failed")
        );
    }
}
