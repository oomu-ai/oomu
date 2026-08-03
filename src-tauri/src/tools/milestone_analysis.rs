use crate::{
    foundation::digest::sha256_hex,
    p0_contracts::EvidenceClass,
    shield_gate::{CommandStatus, ExecuteCommandResponse},
    tools::{
        task_runtime::{record_event, require_agent_runtime_task},
        task_tool_runtime::{
            TaskToolApprovalTier, TaskToolExecutionContext, TaskToolFuture, TaskToolMetadata,
            TaskToolRegistration, TaskToolRiskTier, TaskToolValidation,
        },
    },
};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;

const OPERATION: &str = "analyze_project_milestones";
const MAX_FIXTURE_BYTES: usize = 2 * 1024 * 1024;
const MAX_MILESTONES: usize = 256;
pub(crate) const MAX_ANALYSIS_JSON_BYTES: usize = 6 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AnalyzeProjectMilestonesRequest {
    content: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MilestoneFixtureRecord {
    milestone_id: String,
    name: String,
    target_date: String,
    status: String,
    owner: String,
    #[serde(default)]
    dependencies: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MilestoneAnalysis {
    source_sha256: String,
    milestone_count: usize,
    unfinished_count: usize,
    has_unfinished_milestones: bool,
    milestones: Vec<AnalyzedMilestone>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AnalyzedMilestone {
    milestone_id: String,
    name: String,
    target_date: String,
    status: String,
    owner: String,
    dependencies: Vec<String>,
    unfinished: bool,
}

pub(crate) fn register_task_tool() -> Result<(), String> {
    crate::tools::task_tool_runtime::register(TaskToolRegistration {
        operation: OPERATION,
        validate: validate_registration,
        validate_resolved: validate_registration,
        resolve: crate::tools::task_tool_runtime::identity_resolver,
        execute: execute_registration,
        planner_context: None,
        schema: input_schema,
        metadata: TaskToolMetadata {
            description: "Analyze exact Project milestone fixture bytes deterministically and return a bounded typed unfinished-milestone ledger.",
            risk_tier: TaskToolRiskTier::ReadOnly,
            approval_tier: TaskToolApprovalTier::Background,
            agent_error_code: "project_milestone_analysis_failed",
            agent_error_boundary: "ProjectMilestoneAnalysis",
            execution_path: "The native analyze_project_milestones tool parsed the exact local fixture bytes, validated milestone identities and dependencies, and returned a bounded typed unfinished-work ledger without model inference.",
        },
    })
}

fn input_schema() -> Value {
    json!({
        "type":"object",
        "properties":{
            "content":{
                "type":"string",
                "minLength":1,
                "maxLength":MAX_FIXTURE_BYTES,
                "description":"Exact UTF-8 JSON content returned by the approved Project milestone-file read."
            }
        },
        "required":["content"],
        "additionalProperties":false
    })
}

fn validate_registration(arguments: Value) -> Result<TaskToolValidation, String> {
    let request =
        serde_json::from_value::<AnalyzeProjectMilestonesRequest>(arguments).map_err(|_| {
            "analyze_project_milestones arguments do not match the registered schema.".to_string()
        })?;
    analyze_milestone_fixture(&request.content)?;
    Ok(TaskToolValidation {
        arguments: serde_json::to_value(request).map_err(|error| error.to_string())?,
        potentially_effectful: false,
    })
}

fn execute_registration<'a>(
    context: TaskToolExecutionContext<'a>,
    arguments: Value,
) -> TaskToolFuture<'a> {
    Box::pin(async move {
        let request = serde_json::from_value::<AnalyzeProjectMilestonesRequest>(arguments)
            .map_err(|_| {
                "analyze_project_milestones arguments do not match the registered schema."
                    .to_string()
            })?;
        let execution_id = context
            .execution_id
            .ok_or_else(|| "Milestone analysis requires an active Task.".to_string())?;
        let task = require_agent_runtime_task(context.persistence, execution_id)?;
        let analysis = analyze_milestone_fixture(&request.content)?;
        record_event(
            context.persistence,
            &task.task_run_id,
            "project_milestones.analyzed",
            EvidenceClass::VerifiedPostcondition,
            json!({
                "sourceSha256":analysis["sourceSha256"],
                "milestoneCount":analysis["milestoneCount"],
                "unfinishedCount":analysis["unfinishedCount"],
                "hasUnfinishedMilestones":analysis["hasUnfinishedMilestones"],
            }),
        )?;
        Ok(ExecuteCommandResponse {
            operation: OPERATION.to_string(),
            status: CommandStatus::Completed,
            message: serde_json::to_string(&analysis).map_err(|error| error.to_string())?,
            metrics: None,
            claims: vec![format!(
                "CLAIM project_milestone_analysis=true source_sha256={} milestone_count={} unfinished_count={}",
                analysis["sourceSha256"].as_str().unwrap_or_default(),
                analysis["milestoneCount"].as_u64().unwrap_or_default(),
                analysis["unfinishedCount"].as_u64().unwrap_or_default()
            )],
            verified: true,
            model_used: None,
        })
    })
}

pub(crate) fn analyze_milestone_fixture(content: &str) -> Result<Value, String> {
    if content.trim().is_empty() || content.len() > MAX_FIXTURE_BYTES {
        return Err(
            "Milestone fixture content is empty or exceeds the analysis limit.".to_string(),
        );
    }
    let fixture = serde_json::from_str::<Vec<MilestoneFixtureRecord>>(content)
        .map_err(|_| "Milestone fixture is not a valid milestone JSON array.".to_string())?;
    if fixture.is_empty() || fixture.len() > MAX_MILESTONES {
        return Err(
            "Milestone fixture must contain a bounded, non-empty milestone list.".to_string(),
        );
    }

    let mut ids = HashSet::new();
    let mut milestones = Vec::with_capacity(fixture.len());
    for record in fixture {
        let milestone_id = record.milestone_id.trim().to_string();
        let name = record.name.trim().to_string();
        let target_date = record.target_date.trim().to_string();
        let status = record.status.trim().to_ascii_uppercase();
        let owner = record.owner.trim().to_string();
        let mut dependency_set = HashSet::new();
        let dependencies = record
            .dependencies
            .into_iter()
            .map(|dependency| dependency.trim().to_string())
            .collect::<Vec<_>>();
        if milestone_id.is_empty()
            || milestone_id.len() > 128
            || name.is_empty()
            || name.len() > 512
            || status.is_empty()
            || status.len() > 128
            || owner.is_empty()
            || owner.len() > 256
            || NaiveDate::parse_from_str(&target_date, "%Y-%m-%d").is_err()
            || !ids.insert(milestone_id.clone())
            || dependencies.iter().any(|dependency| {
                dependency.is_empty()
                    || dependency.len() > 128
                    || !dependency_set.insert(dependency.to_ascii_lowercase())
            })
        {
            return Err("Milestone fixture contains an invalid or duplicate record.".to_string());
        }
        milestones.push(AnalyzedMilestone {
            unfinished: status != "COMPLETED",
            milestone_id,
            name,
            target_date,
            status,
            owner,
            dependencies,
        });
    }
    if milestones.iter().any(|milestone| {
        milestone
            .dependencies
            .iter()
            .any(|dependency| !ids.contains(dependency))
    }) {
        return Err("Milestone fixture names an unknown dependency.".to_string());
    }

    let unfinished_count = milestones
        .iter()
        .filter(|milestone| milestone.unfinished)
        .count();
    let analysis = MilestoneAnalysis {
        source_sha256: sha256_hex(content.as_bytes()),
        milestone_count: milestones.len(),
        unfinished_count,
        has_unfinished_milestones: unfinished_count > 0,
        milestones,
    };
    let encoded = serde_json::to_vec(&analysis).map_err(|error| error.to_string())?;
    if encoded.len() > MAX_ANALYSIS_JSON_BYTES {
        return Err(
            "Milestone analysis exceeds the bounded evidence-synthesis contract.".to_string(),
        );
    }
    serde_json::from_slice(&encoded).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"[
      {"milestone_id":"M1","name":"Security isolation","target_date":"2026-07-06","status":"COMPLETED","owner":"Alex"},
      {"milestone_id":"M2","name":"Localization integration","target_date":"2026-07-10","status":"IN_PROGRESS","owner":"Alex","dependencies":["M1"]},
      {"milestone_id":"M3","name":"Release validation","target_date":"2026-07-15","status":"PENDING","owner":"OOMU","dependencies":["M2"]}
    ]"#;

    #[test]
    fn exact_fixture_bytes_drive_bounded_typed_unfinished_milestones() {
        let analysis = analyze_milestone_fixture(FIXTURE).expect("fixture analysis");
        assert_eq!(analysis["milestoneCount"], json!(3));
        assert_eq!(analysis["unfinishedCount"], json!(2));
        assert_eq!(analysis["hasUnfinishedMilestones"], json!(true));
        assert_eq!(analysis["milestones"][0]["unfinished"], json!(false));
        assert_eq!(analysis["milestones"][1]["milestoneId"], json!("M2"));
        assert_eq!(analysis["milestones"][2]["dependencies"], json!(["M2"]));
        assert!(serde_json::to_vec(&analysis).unwrap().len() <= MAX_ANALYSIS_JSON_BYTES);
    }

    #[test]
    fn malformed_dependencies_and_oversized_analysis_are_rejected() {
        assert!(analyze_milestone_fixture("{}").is_err());
        assert!(analyze_milestone_fixture(
            r#"[{"milestone_id":"M1","name":"A","target_date":"2026-07-01","status":"PENDING","owner":"J","dependencies":["missing"]}]"#
        )
        .is_err());
        let oversized = (0..MAX_MILESTONES)
            .map(|index| {
                json!({
                    "milestone_id":format!("M{index}"),
                    "name":"x".repeat(200),
                    "target_date":"2026-07-01",
                    "status":"PENDING",
                    "owner":"Owner"
                })
            })
            .collect::<Vec<_>>();
        assert!(analyze_milestone_fixture(&serde_json::to_string(&oversized).unwrap()).is_err());
    }
}
