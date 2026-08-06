use super::*;

#[test]
fn engine_identity_requires_the_exact_qualified_version_and_build_id() {
    let identity =
        qualified_engine_identity(QUALIFIED_ENGINE_RELEASES[0].executable_digests[0].to_string())
            .unwrap();
    assert_eq!(identity.version, "26.2.4.2");
    let current_identity =
        qualified_engine_identity(QUALIFIED_ENGINE_RELEASES[1].executable_digests[0].to_string())
            .expect("current qualified engine");
    assert_eq!(current_identity.version, "26.2.5.2");
    assert!(qualified_engine_identity("0".repeat(64)).is_err());
}

#[test]
fn checker_setup_uses_one_fixed_official_qualified_release() {
    assert_eq!(QUALIFIED_ENGINE_RELEASE, "26.2.5 (build 26.2.5.2)");
    assert_eq!(
        presentation_checker_download_url(),
        "https://www.libreoffice.org/download/download-libreoffice/?lang=en-US&version=26.2.5"
    );
}
