use std::process::Command;

pub(super) fn failed_state_diff_preview() -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(crate::shield_gate::development_repo_root())
        .arg("diff")
        .arg("--")
        .arg("src")
        .arg("src-tauri")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let diff = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if diff.is_empty() {
        Some("No source diff is currently available for this failed state.".to_string())
    } else {
        Some(super::compact_for_prompt(&diff, 8_000))
    }
}
