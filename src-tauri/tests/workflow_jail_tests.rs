use oomu_lib::workflow_runtime::{
    canonicalize_and_validate_path, is_workflow_environment_allowlisted,
    resolve_workflow_environment_value,
};
use std::path::Path;

#[cfg(unix)]
#[test]
fn test_workflow_rejects_symlink_traversal() {
    let test_root = std::env::temp_dir().join(format!(
        "oomu_workflow_jail_{}_{}",
        std::process::id(),
        unix_time_ms()
    ));
    let sandbox_root = test_root.join("sandbox");
    let outside_root = test_root.join("outside");
    std::fs::create_dir_all(&sandbox_root).unwrap();
    std::fs::create_dir_all(&outside_root).unwrap();
    let secret_path = outside_root.join("secret.txt");
    std::fs::write(&secret_path, "outside jail").unwrap();
    std::os::unix::fs::symlink(&secret_path, sandbox_root.join("linked-secret.txt")).unwrap();

    let error = canonicalize_and_validate_path(&sandbox_root, Path::new("linked-secret.txt"))
        .expect_err("symlink traversal must be blocked");
    assert!(
        error.contains("Access Denied"),
        "unexpected jail error: {error}"
    );

    let _ = std::fs::remove_dir_all(test_root);
}

#[test]
fn test_workflow_rejects_non_allowlisted_env_vars() {
    assert!(is_workflow_environment_allowlisted("PATH"));
    assert!(!is_workflow_environment_allowlisted("OPENAI_API_KEY"));

    let error = resolve_workflow_environment_value("OPENAI_API_KEY")
        .expect_err("non-allowlisted environment reads must be blocked");
    assert_eq!(error.code, "workflow_runtime_permission_rejected");
    assert!(
        error.message.contains("not allowlisted"),
        "unexpected environment error: {}",
        error.message
    );
}

fn unix_time_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}
