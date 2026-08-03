#[path = "../build_identity_policy.rs"]
mod build_identity_policy;

use build_identity_policy::{
    missing_frontend_export_identity, missing_generated_directory_marker, validate_profile,
};
use std::io::ErrorKind;

#[test]
fn release_requires_every_generated_package_input() {
    for path in ["out", "src-tauri/binaries", "src-tauri/resources/python"] {
        assert!(missing_generated_directory_marker("release", path, ErrorKind::NotFound).is_err());
    }
    assert!(missing_frontend_export_identity("release", ErrorKind::NotFound).is_err());
}

#[test]
fn debug_hashes_root_relative_domain_separated_generated_markers() {
    let markers = ["out", "src-tauri/binaries", "src-tauri/resources/python"].map(|path| {
        missing_generated_directory_marker("debug", path, ErrorKind::NotFound)
            .expect("known generated path has a debug marker")
    });
    assert_eq!(
        markers[0],
        "oomu.package-identity.missing-generated-directory.v1\0out"
    );
    assert!(
        markers
            .iter()
            .all(|marker| marker
                .starts_with("oomu.package-identity.missing-generated-directory.v1\0"))
    );
    assert_ne!(markers[0], markers[1]);
    assert_ne!(markers[1], markers[2]);
}

#[test]
fn debug_still_rejects_missing_tracked_or_escaped_paths() {
    assert!(missing_generated_directory_marker(
        "debug",
        "src-tauri/capabilities",
        ErrorKind::NotFound,
    )
    .is_err());
    assert!(missing_generated_directory_marker("debug", "../out", ErrorKind::NotFound).is_err());
    assert!(missing_generated_directory_marker("debug", "/out", ErrorKind::NotFound).is_err());
}

#[test]
fn missing_debug_frontend_has_fixed_empty_export_identity() {
    let identity = missing_frontend_export_identity("debug", ErrorKind::NotFound)
        .expect("debug absence has a deterministic identity");
    assert_eq!(identity.file_count, 0);
    assert_eq!(
        identity.digest,
        "479604b4fc327b923bc0cfc5088439533898f6ed58b15faf492289ac83161182"
    );
}

#[test]
fn unreadable_inputs_and_unknown_profiles_remain_fatal() {
    assert!(
        missing_generated_directory_marker("debug", "out", ErrorKind::PermissionDenied,).is_err()
    );
    assert!(missing_frontend_export_identity("debug", ErrorKind::PermissionDenied).is_err());
    assert!(missing_generated_directory_marker("custom", "out", ErrorKind::NotFound).is_err());
    assert!(missing_frontend_export_identity("custom", ErrorKind::NotFound).is_err());
    assert!(validate_profile("custom").is_err());
}
