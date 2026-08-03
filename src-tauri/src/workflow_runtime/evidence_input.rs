use super::WorkflowRuntimeError;
use serde_json::{Map, Value};

pub(crate) const WORKFLOW_AGENT_VARIABLE_MAX_BYTES: usize = 24 * 1024;

pub(super) fn validate_agent_variables(
    variables: &Map<String, Value>,
) -> Result<(), WorkflowRuntimeError> {
    let has_supplier_analysis = variables.values().any(|value| {
        value.get("supplierCount").is_some()
            && value.get("sourceSha256").is_some()
            && value.get("suppliers").is_some()
    });
    let has_milestone_analysis = variables.values().any(|value| {
        value.get("milestoneCount").is_some()
            && value.get("sourceSha256").is_some()
            && value.get("milestones").is_some()
    });
    let official_sources = variables
        .values()
        .filter_map(official_source_content)
        .collect::<Vec<_>>();
    if official_sources.iter().any(|content| {
        content.chars().count() > crate::tools::official_page::MAX_AGENT_EVIDENCE_CHARS
    }) {
        return Err(WorkflowRuntimeError::new(
            "workflow_official_evidence_too_large",
            format!(
                "This Workflow received more than {} characters from one official source. Fetch a smaller evidence excerpt before synthesis.",
                crate::tools::official_page::MAX_AGENT_EVIDENCE_CHARS
            ),
        ));
    }
    if !has_supplier_analysis && !has_milestone_analysis && official_sources.is_empty() {
        return Ok(());
    }
    let encoded = serde_json::to_vec(variables).map_err(WorkflowRuntimeError::serialization)?;
    if encoded.len() > WORKFLOW_AGENT_VARIABLE_MAX_BYTES {
        return Err(WorkflowRuntimeError::new(
            "workflow_agent_input_too_large",
            format!(
                "This Workflow's evidence input is too large for reliable local synthesis ({} bytes; limit {} bytes). Use typed analysis and smaller source excerpts.",
                encoded.len(),
                WORKFLOW_AGENT_VARIABLE_MAX_BYTES
            ),
        ));
    }
    Ok(())
}

fn official_source_content(value: &Value) -> Option<&str> {
    let object = value.as_object()?;
    let direct = object.get("finalUrl").is_some()
        && object.get("accessedAtUtc").is_some()
        && object.get("contentSha256").is_some();
    if direct {
        return object.get("content").and_then(Value::as_str);
    }
    object
        .get("structuredContent")
        .and_then(official_source_content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn agent_input_budget_accepts_bounded_evidence_and_rejects_overflow() {
        let base = [
            (
                "supplierAnalysis".to_string(),
                json!({"sourceSha256":"a","supplierCount":1,"suppliers":[]}),
            ),
            (
                "milestoneAnalysis".to_string(),
                json!({"sourceSha256":"b","milestoneCount":1,"milestones":[]}),
            ),
        ];
        let bounded = Map::from_iter(base.clone().into_iter().chain([(
            "officialSource".to_string(),
            json!({"finalUrl":"https://example.test","accessedAtUtc":"2026-07-21T12:00:00Z","contentSha256":"a","content":"x".repeat(3_000)}),
        )]));
        validate_agent_variables(&bounded).expect("bounded evidence");

        let oversized = Map::from_iter(base.into_iter().chain([(
            "officialSource".to_string(),
            json!({"finalUrl":"https://example.test","accessedAtUtc":"2026-07-21T12:00:00Z","contentSha256":"a","content":"x".repeat(3_001)}),
        )]));
        let error = validate_agent_variables(&oversized).expect_err("overflow is rejected");
        assert_eq!(error.code, "workflow_official_evidence_too_large");
    }

    #[test]
    fn supplier_exception_evidence_is_bounded_without_milestones() {
        let oversized = Map::from_iter([
            (
                "supplierAnalysis".to_string(),
                json!({"sourceSha256":"a","supplierCount":1,"suppliers":[]}),
            ),
            (
                "officialSource".to_string(),
                json!({"finalUrl":"https://example.test","accessedAtUtc":"2026-07-21T12:00:00Z","contentSha256":"b","content":"x".repeat(3_001)}),
            ),
        ]);
        let error = validate_agent_variables(&oversized)
            .expect_err("Scenario six cannot bypass the per-source ceiling");
        assert_eq!(error.code, "workflow_official_evidence_too_large");
    }

    #[test]
    fn supplier_exception_total_evidence_budget_applies_without_milestones() {
        let mut variables = Map::from_iter([(
            "supplierAnalysis".to_string(),
            json!({"sourceSha256":"a","supplierCount":1,"suppliers":[]}),
        )]);
        for index in 0..9 {
            variables.insert(
                format!("source{index}"),
                json!({"finalUrl":format!("https://example.test/{index}"),"accessedAtUtc":"2026-07-21T12:00:00Z","contentSha256":format!("{index}"),"content":"x".repeat(3_000)}),
            );
        }
        let error = validate_agent_variables(&variables)
            .expect_err("the whole Scenario six evidence set stays bounded");
        assert_eq!(error.code, "workflow_agent_input_too_large");
    }

    #[test]
    fn ordinary_non_evidence_workflows_keep_their_existing_input_behavior() {
        let variables = Map::from_iter([(
            "draft".to_string(),
            json!("x".repeat(WORKFLOW_AGENT_VARIABLE_MAX_BYTES + 1)),
        )]);
        validate_agent_variables(&variables)
            .expect("scenario evidence limits must not narrow ordinary Agent workflows");
    }
}
