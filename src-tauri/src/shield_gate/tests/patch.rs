use super::*;

#[test]
fn unified_diff_hunk_applies_unique_old_block() {
    let original = "alpha\nold\nomega\n";
    let hunk = UnifiedDiffHunk {
        old_block: "alpha\nold\nomega\n".to_string(),
        new_block: "alpha\nnew\nomega\n".to_string(),
    };

    let patched = apply_unified_diff_hunk(original, &hunk).expect("hunk applies");

    assert_eq!(patched, "alpha\nnew\nomega\n");
}

#[test]
fn codebase_compile_authorizes_backend_and_frontend_targets() {
    for (raw_target, expected_target) in [
        ("backend", CodebaseCompileTarget::Backend),
        ("frontend", CodebaseCompileTarget::Frontend),
    ] {
        let action = RequestedAction {
            kind: "codebase_compile".to_string(),
            principal: Some(raw_target.to_string()),
            path: None,
            content: None,
        };

        match authorize_action(action).expect("compile target is classified") {
            AuthorizedActions::CodebaseCompile(request) => {
                assert_eq!(request.target, expected_target);
            }
            other => panic!("expected codebase compile, got {other:?}"),
        }
    }
}

#[test]
fn codebase_compile_rejects_unknown_targets() {
    let action = RequestedAction {
        kind: "codebase_compile".to_string(),
        principal: Some("database".to_string()),
        path: None,
        content: None,
    };

    let rejected = authorize_action(action).expect_err("unknown target is rejected");
    assert_eq!(rejected.code, "security_boundary_violation");
    assert!(rejected.message.contains("backend or frontend"));
}
