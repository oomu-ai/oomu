use crate::{
    foundation::digest::sha256_hex,
    shield_gate::{
        ApprovedExternalDirectoryBinding, ApprovedExternalFileReadBinding, ExecuteCommandResponse,
    },
    tools::task_tool_runtime::{
        TaskToolApprovalTier, TaskToolExecutionContext, TaskToolFuture, TaskToolMetadata,
        TaskToolRegistration, TaskToolRiskTier, TaskToolValidation,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::HashSet, path::Path};

const OPERATION: &str = "create_decision_pack";
const MAX_INPUT_FILES: usize = 8;
const MAX_INPUT_FILE_BYTES: usize = 512 * 1024;
const MAX_TOTAL_INPUT_BYTES: usize = 2 * 1024 * 1024;
const MAX_RESEARCH_QUERIES: usize = 4;

mod analysis;
mod research;
mod runtime;

use crate::decision_research_policy::ResearchPolicy;

pub(super) struct VerifiedInput {
    pub(super) path: String,
    pub(super) sha256: String,
    pub(super) content: zeroize::Zeroizing<String>,
}

#[cfg(test)]
impl VerifiedInput {
    fn test(path: &str, content: &str) -> Self {
        Self {
            path: path.to_string(),
            sha256: sha256_hex(content.as_bytes()),
            content: zeroize::Zeroizing::new(content.to_string()),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct DecisionPackToolRequest {
    title: String,
    locale: String,
    input_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    research_queries: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    research_policy: Option<ResearchPolicy>,
    analysis_instructions: String,
    output_directory: String,
    outputs: DecisionPackOutputs,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    input_bindings: Vec<ApprovedExternalFileReadBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output_binding: Option<ApprovedExternalDirectoryBinding>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct DecisionPackOutputs {
    workbook: String,
    presentation: String,
    pdf: String,
    sources: String,
}

pub(crate) fn register_task_tool() -> Result<(), String> {
    crate::tools::task_tool_runtime::register(TaskToolRegistration {
        operation: OPERATION,
        validate: validate_registration,
        validate_resolved: validate_registration,
        resolve: crate::tools::task_tool_runtime::identity_resolver,
        execute: execute_registration,
        planner_context: None,
        schema: decision_pack_schema,
        metadata: TaskToolMetadata {
            description: "Read approved local supplier evidence, research current official public sources, reconcile amounts and margins deterministically, and export one mutually consistent native XLSX, PPTX, PDF, and Markdown source ledger.",
            risk_tier: TaskToolRiskTier::Network,
            approval_tier: TaskToolApprovalTier::Explicit,
            agent_error_code: "decision_pack_creation_failed",
            agent_error_boundary: "DecisionPack",
            execution_path: "The decision-pack builder bound approved source evidence and live official research to one canonical analysis, generated four native files, and reopened and hashed every output before claiming completion.",
        },
    })
}

fn decision_pack_schema() -> Value {
    json!({
        "type":"object",
        "properties":{
            "title":{"type":"string","minLength":1,"maxLength":240},
            "locale":{"type":"string","minLength":2,"maxLength":35},
            "inputPaths":{"type":"array","minItems":1,"maxItems":MAX_INPUT_FILES,"items":{"type":"string","minLength":1,"maxLength":4096}},
            "researchQueries":{"type":"array","minItems":1,"maxItems":MAX_RESEARCH_QUERIES,"items":{"type":"string","minLength":1,"maxLength":512}},
            "researchPolicy":{
                "type":"object",
                "properties":{
                    "version":{"type":"integer","const":1},
                    "requirement":{"type":"string","enum":["anyOf","allOf"]},
                    "minimumSatisfiedSubjects":{"type":"integer","minimum":1,"maximum":2},
                    "subjects":{"type":"array","minItems":1,"maxItems":2,"items":{
                        "type":"object",
                        "properties":{
                            "subject":{"type":"string","enum":["fuel","freight"]},
                            "queryAlternatives":{"type":"array","minItems":1,"maxItems":3,"items":{
                                "type":"object",
                                "properties":{
                                    "query":{"type":"string","minLength":1,"maxLength":512},
                                    "authorityProfile":{"type":"string","minLength":1,"maxLength":120}
                                },
                                "required":["query","authorityProfile"],
                                "additionalProperties":false
                            }}
                        },
                        "required":["subject","queryAlternatives"],
                        "additionalProperties":false
                    }}
                },
                "required":["version","requirement","minimumSatisfiedSubjects","subjects"],
                "additionalProperties":false
            },
            "analysisInstructions":{"type":"string","minLength":1,"maxLength":8000},
            "outputDirectory":{"type":"string","minLength":1,"maxLength":4096},
            "outputs":{"type":"object","properties":{
                "workbook":{"type":"string","pattern":"^[^/\\\\]+\\.xlsx$"},
                "presentation":{"type":"string","pattern":"^[^/\\\\]+\\.pptx$"},
                "pdf":{"type":"string","pattern":"^[^/\\\\]+\\.pdf$"},
                "sources":{"type":"string","pattern":"^[^/\\\\]+\\.md$"}
            },"required":["workbook","presentation","pdf","sources"],"additionalProperties":false}
        },
        "required":["title","locale","inputPaths","analysisInstructions","outputDirectory","outputs"],
        "oneOf":[
            {"required":["researchPolicy"],"not":{"required":["researchQueries"]}},
            {"required":["researchQueries"],"not":{"required":["researchPolicy"]}}
        ],
        "additionalProperties":false
    })
}

fn validate_registration(arguments: Value) -> Result<TaskToolValidation, String> {
    let mut request =
        serde_json::from_value::<DecisionPackToolRequest>(arguments).map_err(|_| {
            "create_decision_pack arguments do not match the registered schema.".to_string()
        })?;
    normalize_request(&mut request)?;
    validate_and_bind_request(&mut request)?;
    Ok(TaskToolValidation {
        arguments: serde_json::to_value(request).map_err(|error| error.to_string())?,
        potentially_effectful: true,
    })
}

fn normalize_request(request: &mut DecisionPackToolRequest) -> Result<(), String> {
    request.title = bounded_text(&request.title, 1, 240, "title")?;
    request.locale = bounded_text(&request.locale, 2, 35, "locale")?;
    request.analysis_instructions = bounded_text(
        &request.analysis_instructions,
        1,
        8_000,
        "analysis instructions",
    )?;
    request.output_directory = bounded_text(&request.output_directory, 1, 4_096, "output folder")?;
    for path in &mut request.input_paths {
        *path = bounded_text(path, 1, 4_096, "input path")?;
    }
    for query in &mut request.research_queries {
        *query = bounded_text(query, 1, 512, "research query")?;
    }
    if let Some(policy) = &request.research_policy {
        for subject in &policy.subjects {
            for alternative in &subject.query_alternatives {
                if bounded_text(&alternative.query, 1, 512, "research query")? != alternative.query
                    || bounded_text(
                        &alternative.authority_profile,
                        1,
                        120,
                        "research authority profile",
                    )? != alternative.authority_profile
                {
                    return Err(
                        "create_decision_pack research policy cannot be normalized after approval."
                            .to_string(),
                    );
                }
            }
        }
        crate::decision_research_policy::validate_research_policy(policy)?;
    }
    for name in [
        &mut request.outputs.workbook,
        &mut request.outputs.presentation,
        &mut request.outputs.pdf,
        &mut request.outputs.sources,
    ] {
        *name = bounded_text(name, 1, 240, "output filename")?;
    }
    Ok(())
}

fn bounded_text(
    value: &str,
    minimum: usize,
    maximum: usize,
    field: &str,
) -> Result<String, String> {
    let value = value.trim();
    if value.contains('\0') || !(minimum..=maximum).contains(&value.chars().count()) {
        return Err(format!(
            "create_decision_pack {field} is outside its bounded contract."
        ));
    }
    Ok(value.to_string())
}

fn validate_and_bind_request(request: &mut DecisionPackToolRequest) -> Result<(), String> {
    let uses_legacy_research = !request.research_queries.is_empty();
    let uses_structured_research = request.research_policy.is_some();
    if request.input_paths.is_empty()
        || request.input_paths.len() > MAX_INPUT_FILES
        || request.research_queries.len() > MAX_RESEARCH_QUERIES
        || uses_legacy_research == uses_structured_research
    {
        return Err("create_decision_pack request is outside its bounded contract.".to_string());
    }
    let mut paths = HashSet::new();
    let rebound_inputs = request
        .input_paths
        .iter()
        .map(|path| {
            crate::shield_gate::bind_approved_external_file_read(path)
                .map_err(|error| error.message)
        })
        .collect::<Result<Vec<_>, _>>()?;
    for binding in &rebound_inputs {
        if !paths.insert(binding.canonical_path.clone()) {
            return Err("create_decision_pack input paths must be distinct.".to_string());
        }
    }
    if !request.input_bindings.is_empty() && request.input_bindings != rebound_inputs {
        return Err(
            "An approved decision-pack input changed before execution. Nothing was changed."
                .to_string(),
        );
    }
    request.input_bindings = rebound_inputs;
    let rebound_output =
        crate::shield_gate::bind_approved_external_directory_creation(&request.output_directory)
            .map_err(|error| error.message)?;
    if request
        .output_binding
        .as_ref()
        .is_some_and(|binding| binding != &rebound_output)
    {
        return Err(
            "The approved decision-pack output folder changed before execution. Nothing was changed."
                .to_string(),
        );
    }
    request.output_directory = rebound_output.canonical_path.clone();
    request.output_binding = Some(rebound_output);
    let mut outputs = HashSet::new();
    for (name, extension) in [
        (&request.outputs.workbook, "xlsx"),
        (&request.outputs.presentation, "pptx"),
        (&request.outputs.pdf, "pdf"),
        (&request.outputs.sources, "md"),
    ] {
        validate_filename(name, extension)?;
        if !outputs.insert(name.to_ascii_lowercase()) {
            return Err("create_decision_pack output filenames must be distinct.".to_string());
        }
        crate::shield_gate::validate_approved_external_write_target(
            &Path::new(&request.output_directory)
                .join(name)
                .to_string_lossy(),
        )
        .map_err(|error| error.message)?;
    }
    Ok(())
}

fn validate_filename(value: &str, extension: &str) -> Result<(), String> {
    let path = Path::new(value);
    if path.components().count() != 1
        || path.file_name().and_then(|name| name.to_str()) != Some(value)
        || path.extension().and_then(|value| value.to_str()) != Some(extension)
    {
        return Err(format!(
            "create_decision_pack requires a .{extension} filename."
        ));
    }
    Ok(())
}

fn execute_registration<'a>(
    context: TaskToolExecutionContext<'a>,
    arguments: Value,
) -> TaskToolFuture<'a> {
    Box::pin(async move {
        let request =
            serde_json::from_value::<DecisionPackToolRequest>(arguments).map_err(|_| {
                "create_decision_pack arguments do not match the registered schema.".to_string()
            })?;
        execute_decision_pack(context, request).await
    })
}

async fn execute_decision_pack(
    context: TaskToolExecutionContext<'_>,
    request: DecisionPackToolRequest,
) -> Result<ExecuteCommandResponse, String> {
    runtime::execute(context, request).await
}

#[allow(dead_code)]
fn request_digest(request: &DecisionPackToolRequest) -> Result<String, String> {
    serde_json::to_vec(request)
        .map(|bytes| sha256_hex(&bytes))
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_digest_is_stable_for_the_same_request() {
        let request = DecisionPackToolRequest {
            title: "Supplier decision".to_string(),
            locale: "en-US".to_string(),
            input_paths: vec!["/tmp/input.json".to_string()],
            research_queries: vec!["site:eia.gov diesel prices".to_string()],
            research_policy: None,
            analysis_instructions: "Reconcile every amount.".to_string(),
            output_directory: "/tmp/output".to_string(),
            outputs: DecisionPackOutputs {
                workbook: "decision.xlsx".to_string(),
                presentation: "decision.pptx".to_string(),
                pdf: "decision.pdf".to_string(),
                sources: "sources.md".to_string(),
            },
            input_bindings: Vec::new(),
            output_binding: None,
        };
        assert_eq!(
            request_digest(&request).unwrap(),
            request_digest(&request).unwrap()
        );
    }

    #[test]
    fn legacy_research_queries_round_trip_without_silent_policy_migration() {
        let value = json!({
            "title":"Supplier Decision Pack",
            "locale":"en-US",
            "inputPaths":["/tmp/input.json"],
            "researchQueries":["official fuel conditions"],
            "analysisInstructions":"Reconcile every amount and margin and identify every exception.",
            "outputDirectory":"/tmp/output",
            "outputs":{
                "workbook":"decision.xlsx",
                "presentation":"decision.pptx",
                "pdf":"decision.pdf",
                "sources":"sources.md"
            }
        });
        let request: DecisionPackToolRequest = serde_json::from_value(value.clone()).unwrap();
        let encoded = serde_json::to_value(request).unwrap();
        assert_eq!(encoded.get("researchQueries"), value.get("researchQueries"));
        assert!(encoded.get("researchPolicy").is_none());
    }

    #[test]
    fn planner_validation_injects_immutable_input_and_output_bindings() {
        let root = std::env::temp_dir().join(format!(
            "oomu-decision-pack-bindings-{}-{}",
            std::process::id(),
            crate::foundation::clock::unix_time_ns_u128()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        let first = root.join("rates.json");
        let second = root.join("margins.txt");
        std::fs::write(&first, "{}\n").unwrap();
        std::fs::write(&second, "margin\n").unwrap();
        let output = root.join("decision_output");
        let validated = validate_registration(json!({
            "title":"Supplier Decision Pack",
            "locale":"en-US",
            "inputPaths":[first.to_string_lossy(), second.to_string_lossy()],
            "researchQueries":["official fuel conditions"],
            "analysisInstructions":"Reconcile every amount and margin and identify every exception.",
            "outputDirectory":output.to_string_lossy(),
            "outputs":{
                "workbook":"decision.xlsx",
                "presentation":"decision.pptx",
                "pdf":"decision.pdf",
                "sources":"sources.md"
            }
        }))
        .expect("registered planner arguments should bind exact filesystem identities");

        assert_eq!(
            validated
                .arguments
                .get("inputBindings")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(2)
        );
        assert!(validated.arguments.get("outputBinding").is_some());
        std::fs::remove_dir_all(root).unwrap();
    }
}
