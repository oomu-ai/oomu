use super::path_scope::single_active_project_root;
use crate::{
    agentic_loop::{ActionPlan, AgenticLoopError, Step, Tool},
    db::PersistenceEngine,
};

pub(crate) fn bind_missing_terminal_cwds(
    engine: &PersistenceEngine,
    project_id: Option<&str>,
    steps: &mut [Step],
) -> Result<(), AgenticLoopError> {
    if !steps
        .iter()
        .any(|step| matches!(step.tool, Tool::TerminalExecute { cwd: None, .. }))
    {
        return Ok(());
    }
    let project_id = project_id.ok_or_else(|| AgenticLoopError {
        code: "project_root_required",
        boundary: "ProjectTerminalScope",
        message: "Choose a Project folder before running terminal work without a working folder."
            .to_string(),
        mlc_path: None,
    })?;
    let root =
        single_active_project_root(engine, project_id).map_err(|message| AgenticLoopError {
            code: "project_root_required",
            boundary: "ProjectTerminalScope",
            message,
            mlc_path: None,
        })?;
    let cwd = root.to_string_lossy().to_string();
    for step in steps {
        if let Tool::TerminalExecute {
            cwd: target @ None, ..
        } = &mut step.tool
        {
            *target = Some(cwd.clone());
        }
    }
    Ok(())
}

pub(crate) fn bind_plan_terminal_cwds(
    engine: &PersistenceEngine,
    project_id: &Option<String>,
    mut plan: ActionPlan,
) -> Result<ActionPlan, AgenticLoopError> {
    bind_missing_terminal_cwds(engine, project_id.as_deref(), &mut plan.steps)?;
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agentic_loop::RiskLevel,
        projects::{repository, CreateProjectRequest, ProjectDataPolicy},
    };
    use std::{collections::BTreeMap, fs};

    fn terminal(cwd: Option<String>) -> Step {
        Step {
            step: "Inspect the Project".to_string(),
            tool: Tool::TerminalExecute {
                executable: "pwd".to_string(),
                args: Vec::new(),
                env: BTreeMap::new(),
                cwd,
                timeout: None,
            },
            risk_level: RiskLevel::Low,
        }
    }

    fn terminal_cwd(step: &Step) -> Option<&str> {
        match &step.tool {
            Tool::TerminalExecute { cwd, .. } => cwd.as_deref(),
            _ => None,
        }
    }

    #[test]
    fn missing_cwd_binds_before_signing_while_explicit_cwd_is_preserved() {
        let root = std::env::temp_dir().join(format!(
            "oomu-project-terminal-scope-{}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        let selected = root.join("selected");
        fs::create_dir_all(&selected).unwrap();
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let project = repository::create(
            &engine,
            CreateProjectRequest {
                name: "Terminal root".to_string(),
                description: String::new(),
                data_policy: ProjectDataPolicy::LocalOnly,
            },
        )
        .unwrap();
        repository::attach_picked_root(&engine, &project.project_id, &selected).unwrap();
        let explicit = root.join("explicit").to_string_lossy().to_string();
        let mut steps = vec![terminal(None), terminal(Some(explicit.clone()))];

        bind_missing_terminal_cwds(&engine, Some(&project.project_id), &mut steps).unwrap();

        let selected = fs::canonicalize(selected)
            .unwrap()
            .to_string_lossy()
            .to_string();
        assert_eq!(terminal_cwd(&steps[0]), Some(selected.as_str()));
        assert_eq!(terminal_cwd(&steps[1]), Some(explicit.as_str()));
        let _ = fs::remove_dir_all(root);
    }
}
