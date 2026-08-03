use super::*;
use std::collections::HashSet;

const READ_ONLY_TOOLS: [&str; 6] = [
    "project_file_read",
    "web_search",
    "connector_read",
    "browser_snapshot",
    "task_evidence_read",
    "summarize_text",
];
const FORBIDDEN_MARKERS: [&str; 17] = [
    "write",
    "delete",
    "remove",
    "shell",
    "exec",
    "command",
    "post",
    "send",
    "create_event",
    "update_event",
    "approval",
    "grant",
    "export",
    "upload",
    "download",
    "delegate",
    "spawn",
];

pub(crate) fn validate(request: &CreateDelegationPlanRequest) -> Result<(), String> {
    if request.schema_version != DELEGATION_SCHEMA_VERSION {
        return Err("Unsupported delegation contract version.".into());
    }
    if request.parent_depth != 0 {
        return Err("Nested delegation is not supported; child depth is limited to one.".into());
    }
    if request.children.is_empty() {
        return Err("A delegation plan requires at least one child.".into());
    }
    if request.children.len() > MAX_PARALLEL_CHILDREN {
        return Err(format!("Delegation proposal contains {} children; the enforced maximum is {MAX_PARALLEL_CHILDREN}.", request.children.len()));
    }
    validate_aggregate(&request.aggregate_budget)?;
    let mut total_input = 0usize;
    let mut total_output = 0usize;
    let mut total_tools = 0usize;
    for child in &request.children {
        validate_child(child)?;
        total_input = total_input.saturating_add(child.budget.max_input_tokens);
        total_output = total_output.saturating_add(child.budget.max_output_tokens);
        total_tools = total_tools.saturating_add(child.budget.max_tool_calls);
    }
    if total_input > request.aggregate_budget.max_input_tokens
        || total_output > request.aggregate_budget.max_output_tokens
        || total_tools > request.aggregate_budget.max_tool_calls
    {
        return Err("Child budgets exceed the aggregate delegation budget.".into());
    }
    Ok(())
}

fn validate_aggregate(budget: &AggregateBudget) -> Result<(), String> {
    if !(128..=256_000).contains(&budget.max_input_tokens)
        || !(32..=64_000).contains(&budget.max_output_tokens)
        || !(1..=64).contains(&budget.max_tool_calls)
        || !(1_000..=600_000).contains(&budget.timeout_ms)
    {
        return Err("Aggregate delegation budget is outside bounded V1 limits.".into());
    }
    Ok(())
}

fn validate_child(child: &ChildProposal) -> Result<(), String> {
    if child.goal.trim().len() < 3 || child.goal.chars().count() > 2_000 {
        return Err("Each child requires a bounded goal.".into());
    }
    if child.expected_output_schema != "findings_sources_uncertainties_v1" {
        return Err("Unsupported child output schema.".into());
    }
    if child.sources.is_empty() || child.sources.len() > 8 {
        return Err("Each child requires one to eight explicit sources.".into());
    }
    if child.allowed_read_tools.is_empty() || child.allowed_read_tools.len() > 8 {
        return Err("Each child requires bounded read-only tools.".into());
    }
    let mut tools = HashSet::new();
    for raw in &child.allowed_read_tools {
        let tool = raw.trim().to_ascii_lowercase();
        if mandatory_parent_only_action(&tool) || !READ_ONLY_TOOLS.contains(&tool.as_str()) {
            return Err(format!(
                "Child capability policy denied non-read-only tool: {raw}."
            ));
        }
        if !tools.insert(tool) {
            return Err("Child tool scope contains duplicates.".into());
        }
    }
    for source in &child.sources {
        let required = match source {
            DelegatedSource::InlineText { content, .. } => {
                if content.is_empty() || content.len() > 512_000 {
                    return Err("Inline child context is outside the size limit.".into());
                }
                "summarize_text"
            }
            DelegatedSource::ProjectFile { relative_path, .. } => {
                if relative_path.contains("..") || relative_path.starts_with('/') {
                    return Err("Project file delegation requires a safe relative path.".into());
                }
                "project_file_read"
            }
            DelegatedSource::WebSearch {
                query,
                max_results,
                authorization,
            } => {
                if query.trim().is_empty() || query.len() > 500 || max_results.unwrap_or(5) > 5 {
                    return Err("Web delegation source is invalid.".into());
                }
                if authorization.originating_user_objective.trim().is_empty()
                    || authorization.originating_user_objective.chars().count() > 16_000
                    || !crate::sovereign_search::delegated_search_authorization_is_valid(
                        &authorization.originating_user_objective,
                        &authorization.approved_query,
                        query,
                    )
                {
                    return Err(
                        "Web delegation requires an explicit user search request bound to the exact query."
                            .into(),
                    );
                }
                "web_search"
            }
            DelegatedSource::BrowserSnapshot { .. } => "browser_snapshot",
            DelegatedSource::TaskEvidence { event_types } => {
                if event_types.is_empty() || event_types.len() > 12 {
                    return Err("Task evidence source is invalid.".into());
                }
                "task_evidence_read"
            }
        };
        if !tools.contains(required) {
            return Err(format!(
                "Source {} requires delegated tool {required}.",
                source.kind()
            ));
        }
    }
    let budget = &child.budget;
    if !(64..=32_000).contains(&budget.max_input_tokens)
        || !(16..=8_000).contains(&budget.max_output_tokens)
        || budget.max_tool_calls < child.sources.len()
        || budget.max_tool_calls > 8
        || !(1_000..=300_000).contains(&budget.timeout_ms)
        || !(1_024..=1_048_576).contains(&budget.max_response_bytes)
    {
        return Err("Child resource budget is outside bounded V1 limits.".into());
    }
    if !matches!(child.model_route.trim(), "parent" | "local" | "local_gemma") {
        return Err(
            "V1 child model route must be parent or an approved credential-isolated local route."
                .into(),
        );
    }
    Ok(())
}

pub(crate) fn validate_child_template(child: &ChildProposal) -> Result<(), String> {
    validate_child(child)
}

pub(crate) fn mandatory_parent_only_action(action: &str) -> bool {
    FORBIDDEN_MARKERS
        .iter()
        .any(|marker| action.trim().to_ascii_lowercase().contains(marker))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn child(tool: &str) -> ChildProposal {
        ChildProposal {
            goal: "Inspect evidence".into(),
            expected_output_schema: "findings_sources_uncertainties_v1".into(),
            sources: vec![DelegatedSource::InlineText {
                label: "note".into(),
                content: "evidence".into(),
            }],
            allowed_read_tools: vec![tool.into()],
            model_route: "local".into(),
            budget: ResourceBudget {
                max_input_tokens: 256,
                max_output_tokens: 128,
                max_tool_calls: 1,
                timeout_ms: 10_000,
                max_response_bytes: 16_384,
            },
        }
    }
    #[test]
    fn rejects_over_limit_instead_of_truncating() {
        let mut request = CreateDelegationPlanRequest {
            schema_version: 1,
            project_id: "project_invalid".into(),
            task_run_id: "taskrun_invalid".into(),
            parent_session_id: None,
            parent_model_route: "local".into(),
            parent_depth: 0,
            aggregate_budget: AggregateBudget {
                max_input_tokens: 4096,
                max_output_tokens: 2048,
                max_tool_calls: 8,
                timeout_ms: 20_000,
            },
            children: vec![child("summarize_text"); 9],
        };
        assert!(validate(&request).unwrap_err().contains("maximum"));
        request.children.truncate(8);
        assert!(validate(&request).is_ok());
    }
    #[test]
    fn rejects_writes_nested_delegation_and_authority() {
        for tool in [
            "file_write",
            "shell_command",
            "connector_post",
            "approval_grant",
            "delegate",
        ] {
            assert!(validate_child(&child(tool)).is_err());
        }
        assert!(mandatory_parent_only_action("artifact_export"));
    }

    #[test]
    fn delegated_web_search_requires_explicit_objective_and_exact_query_binding() {
        let web_child = |objective: &str, approved_query: &str, query: &str| ChildProposal {
            goal: "Compare current public release notes".into(),
            expected_output_schema: "findings_sources_uncertainties_v1".into(),
            sources: vec![DelegatedSource::WebSearch {
                query: query.into(),
                max_results: Some(5),
                authorization: DelegatedWebSearchAuthorization {
                    originating_user_objective: objective.into(),
                    approved_query: approved_query.into(),
                },
            }],
            allowed_read_tools: vec!["web_search".into()],
            model_route: "local".into(),
            budget: ResourceBudget {
                max_input_tokens: 256,
                max_output_tokens: 128,
                max_tool_calls: 1,
                timeout_ms: 10_000,
                max_response_bytes: 16_384,
            },
        };

        assert!(validate_child(&web_child(
            "Search online for Rust release notes",
            "Rust release notes",
            "Rust release notes",
        ))
        .is_ok());
        assert!(validate_child(&web_child(
            "Find Rust release notes",
            "Rust release notes",
            "Rust release notes",
        ))
        .is_err());
        assert!(validate_child(&web_child(
            "Search online for Rust release notes",
            "Rust release notes",
            "private account records",
        ))
        .is_err());
        assert!(validate_child(&web_child(
            "Search online for my iMessages",
            "my iMessages",
            "my iMessages",
        ))
        .is_err());
    }
}
