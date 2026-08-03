use super::*;
use std::collections::BTreeMap;

pub(super) fn compile(objective: &str) -> Option<GeneratedActionPlanDraft> {
    if !super::super::is_explicit_read_only_project_status_request(objective) {
        return None;
    }
    let root = crate::shield_gate::development_repo_root()
        .canonicalize()
        .unwrap_or_else(|_| crate::shield_gate::development_repo_root());
    let tool = GeneratedToolDraft::TerminalExecute {
        executable: "/usr/bin/git".to_string(),
        args: vec![
            "status".to_string(),
            "--short".to_string(),
            "--branch".to_string(),
        ],
        env: BTreeMap::new(),
        cwd: Some(root.to_string_lossy().into_owned()),
        timeout: Some(crate::tools::terminal_contract::DEFAULT_TERMINAL_TIMEOUT_MS),
    };
    let step = GeneratedPlanStepDraft {
        step: "Check the current project for uncommitted changes without modifying it.".to_string(),
        tool,
        risk_level: GeneratedRiskLevel::Low,
    };
    let exit_condition = "Exit only after the read-only Git status command finishes and its verified result is reported to the user."
        .to_string();
    let generated_text = serde_json::json!({
        "steps": [&step],
        "exit_condition": &exit_condition,
    })
    .to_string();
    Some(GeneratedActionPlanDraft {
        steps: vec![step],
        exit_condition,
        generated_text,
        source: IntentSource::Deterministic,
        degraded_reason: None,
    })
}

pub(super) fn validate(
    objective: &str,
    draft: &GeneratedActionPlanDraft,
) -> Result<(), PlanCoverageDeficit> {
    let Some(expected) = compile(objective) else {
        return Ok(());
    };
    if serde_json::to_value(&draft.steps).ok() != serde_json::to_value(&expected.steps).ok()
        || draft.exit_condition != expected.exit_condition
    {
        return Err(PlanCoverageDeficit::missing(vec![
            "verified read-only current-project status".to_string(),
        ]));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROMPT: &str = "Inspect the current OOMU project and tell me whether its working tree has changes. Do not modify anything.";

    #[test]
    fn exact_project_status_request_compiles_without_model_planning() {
        let draft = compile(PROMPT).expect("deterministic project-status draft");
        assert_eq!(draft.steps.len(), 1);
        assert!(matches!(
            &draft.steps[0].tool,
            GeneratedToolDraft::TerminalExecute { executable, args, env, cwd: Some(cwd), timeout: Some(timeout) }
                if executable == "/usr/bin/git"
                    && args == &["status", "--short", "--branch"]
                    && env.is_empty()
                    && !cwd.trim().is_empty()
                    && *timeout == crate::tools::terminal_contract::DEFAULT_TERMINAL_TIMEOUT_MS
        ));
        assert!(matches!(draft.source, IntentSource::Deterministic));
        validate(PROMPT, &draft).expect("exact deterministic draft validates");
    }

    #[test]
    fn informational_git_question_does_not_compile_an_action() {
        assert!(compile("What does a Git working tree mean?").is_none());
    }
}
