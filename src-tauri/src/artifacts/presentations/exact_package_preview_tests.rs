use super::*;

#[test]
fn engine_identity_requires_the_exact_qualified_version_and_build_id() {
    let brand = QUALIFIED_ENGINE_BRAND;
    let accepted = format!(
        "{brand} 26.2.4.2 {}\n",
        QUALIFIED_ENGINE_RELEASES[0].build_id
    );
    let identity = parse_engine_identity(
        accepted.as_bytes(),
        QUALIFIED_ENGINE_RELEASES[0].executable_digests[0].to_string(),
    )
    .unwrap();
    assert_eq!(identity.version, "26.2.4.2");
    let current = format!(
        "{brand} 26.2.5.2 {}\n",
        QUALIFIED_ENGINE_RELEASES[1].build_id
    );
    let current_identity = parse_engine_identity(
        current.as_bytes(),
        QUALIFIED_ENGINE_RELEASES[1].executable_digests[0].to_string(),
    )
    .expect("current qualified engine");
    assert_eq!(current_identity.version, "26.2.5.2");
    let mixed = format!(
        "{brand} 26.2.4.2 {}\n",
        QUALIFIED_ENGINE_RELEASES[1].build_id
    );
    assert!(parse_engine_identity(
        mixed.as_bytes(),
        QUALIFIED_ENGINE_RELEASES[1].executable_digests[0].to_string(),
    )
    .is_err());

    let unqualified = format!(
        "{brand} 26.2.6.1 {}\n",
        QUALIFIED_ENGINE_RELEASES[0].build_id
    );
    assert!(parse_engine_identity(
        unqualified.as_bytes(),
        QUALIFIED_ENGINE_RELEASES[0].executable_digests[0].to_string()
    )
    .is_err());
    let bad_build = format!("{brand} 26.2.4.2 not-a-build\n");
    assert!(parse_engine_identity(
        bad_build.as_bytes(),
        QUALIFIED_ENGINE_RELEASES[0].executable_digests[0].to_string()
    )
    .is_err());
    assert!(parse_engine_identity(accepted.as_bytes(), "0".repeat(64)).is_err());
}

#[test]
fn checker_setup_uses_one_fixed_official_qualified_release() {
    assert_eq!(QUALIFIED_ENGINE_RELEASE, "26.2.5 (build 26.2.5.2)");
    assert_eq!(
        presentation_checker_download_url(),
        "https://www.libreoffice.org/download/download-libreoffice/?lang=en-US&version=26.2.5"
    );
}
