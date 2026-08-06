use crate::gemma::{
    GeneratedActionPlanDraft, GeneratedPlanStepDraft, GeneratedRiskLevel, GeneratedToolDraft,
    IntentSource,
};
use regex::Regex;
use serde_json::Value;
use std::{collections::HashSet, path::Path, sync::OnceLock};
mod compound_requirements;
mod contextual_path;
mod decision_pack_contract;
mod evidence_artifact_contract;
mod project_status_contract;
mod release_recovery_contract;
mod specialist_composition;
mod specialist_output;
pub(super) use contextual_path::requests_contextual_path_grounding;
pub(super) fn matches_deterministic_decision_pack_plan(
    plan: &super::ActionPlan,
    trusted_output_directory: &str,
) -> bool {
    let Ok(Some(expected)) =
        decision_pack_contract::compile(&plan.objective, Some(trusted_output_directory))
    else {
        return false;
    };
    if plan.steps.len() < expected.steps.len()
        || serde_json::to_value(&plan.steps[..expected.steps.len()]).ok()
            != serde_json::to_value(&expected.steps).ok()
    {
        return false;
    }
    specialist_composition::exit_condition_matches(
        &plan.exit_condition,
        &expected.exit_condition,
        plan.steps.len() > expected.steps.len(),
    )
}

pub(super) fn matches_deterministic_release_recovery_plan(plan: &super::ActionPlan) -> bool {
    release_recovery_contract::matches_runtime_plan(plan)
}

pub(super) fn release_recovery_requested_calendar_name(objective: &str) -> Option<String> {
    release_recovery_contract::requested_calendar_name(objective)
}

const FILE_EXTENSIONS: &[&str] = &[
    "c", "cpp", "csv", "db", "doc", "docx", "gif", "go", "gz", "h", "hpp", "htm", "html", "java",
    "jpeg", "jpg", "js", "json", "jsx", "kt", "md", "markdown", "pdf", "png", "ppt", "pptx", "py",
    "rb", "rs", "rtf", "sh", "sql", "sqlite", "svg", "swift", "tar", "toml", "ts", "tsv", "tsx",
    "txt", "webp", "xls", "xlsx", "xml", "yaml", "yml", "zip", "zsh",
];
const TEXT_FILE_FORMATS: &[&str] = &[
    "c", "cpp", "csv", "go", "h", "hpp", "htm", "html", "java", "js", "json", "jsx", "kt", "md",
    "markdown", "py", "rb", "rs", "rtf", "sh", "sql", "swift", "toml", "ts", "tsv", "tsx", "txt",
    "xml", "yaml", "yml", "zsh",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PlanCoverageDeficit {
    pub(super) missing: Vec<String>,
    kind: PlanCoverageDeficitKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PlanCoverageDeficitKind {
    MissingRequirements,
    DecisionPackContract,
    DecisionPackCalendarRequired,
    DecisionPackInputPath,
    ReleaseRecoveryContract,
}

impl PlanCoverageDeficit {
    fn missing(missing: Vec<String>) -> Self {
        Self {
            missing,
            kind: PlanCoverageDeficitKind::MissingRequirements,
        }
    }

    fn decision_pack_contract(problem: impl Into<String>) -> Self {
        Self {
            missing: vec![problem.into()],
            kind: PlanCoverageDeficitKind::DecisionPackContract,
        }
    }

    fn decision_pack_calendar_required() -> Self {
        Self {
            missing: vec!["The requested Calendar name was not explicit.".to_string()],
            kind: PlanCoverageDeficitKind::DecisionPackCalendarRequired,
        }
    }

    fn decision_pack_input_path(problem: impl Into<String>) -> Self {
        Self {
            missing: vec![problem.into()],
            kind: PlanCoverageDeficitKind::DecisionPackInputPath,
        }
    }

    fn release_recovery_contract(problem: impl Into<String>) -> Self {
        Self {
            missing: vec![problem.into()],
            kind: PlanCoverageDeficitKind::ReleaseRecoveryContract,
        }
    }

    pub(super) fn code(&self) -> &'static str {
        match self.kind {
            PlanCoverageDeficitKind::MissingRequirements => "planner_objective_coverage_incomplete",
            PlanCoverageDeficitKind::DecisionPackContract => {
                "planner_decision_pack_contract_invalid"
            }
            PlanCoverageDeficitKind::DecisionPackCalendarRequired => {
                "planner_decision_pack_calendar_required"
            }
            PlanCoverageDeficitKind::DecisionPackInputPath => {
                "planner_decision_pack_input_path_invalid"
            }
            PlanCoverageDeficitKind::ReleaseRecoveryContract => {
                "planner_release_recovery_contract_invalid"
            }
        }
    }

    pub(super) fn message(&self) -> String {
        match self.kind {
            PlanCoverageDeficitKind::MissingRequirements => format!(
                "The planner did not cover every explicitly requested deliverable or action. Missing: {}. No action was executed.",
                self.missing.join("; ")
            ),
            PlanCoverageDeficitKind::DecisionPackContract => format!(
                "OOMU rejected the generated plan because it did not match the requested decision-pack, conflict-free Calendar, and unsent Mail contract. {} No action was executed.",
                self.missing.join("; ")
            ),
            PlanCoverageDeficitKind::DecisionPackCalendarRequired =>
                "Choose the Calendar that should hold the Supplier Decision Review event. No action was executed."
                    .to_string(),
            PlanCoverageDeficitKind::DecisionPackInputPath => format!(
                "OOMU could not safely bind the requested decision-pack input files. {} Check the path spelling and try again. No action was executed.",
                self.missing.join("; ")
            ),
            PlanCoverageDeficitKind::ReleaseRecoveryContract => format!(
                "OOMU could not safely bind the recovery agenda, Calendar event, and Mail draft into one exact workflow. {} No action was executed.",
                self.missing.join("; ")
            ),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Requirement {
    InputFile { path: String },
    InputDirectory { path: String },
    OutputFile { path: String, format: String },
    OutputFormat { label: String, format: String },
    ExternalResearch,
    CrossSurface(compound_requirements::CrossSurfaceRequirement),
}

impl Requirement {
    fn label(&self) -> String {
        match self {
            Self::InputFile { path } => format!("input file read '{path}'"),
            Self::InputDirectory { path } => format!("input directory listing '{path}'"),
            Self::OutputFile { path, .. } => format!("output file '{path}'"),
            Self::OutputFormat { label, .. } => format!("output format '{label}'"),
            Self::ExternalResearch => "external web research".to_string(),
            Self::CrossSurface(requirement) => requirement.label(),
        }
    }

    fn planner_clause(&self) -> String {
        match self {
            Self::InputFile { path } => format!("Read the exact input file `{path}`."),
            Self::InputDirectory { path } => {
                format!("List the exact input directory `{path}`.")
            }
            Self::OutputFile { path, .. } => {
                format!("Create and verify the exact additional output file `{path}`.")
            }
            Self::OutputFormat { label, .. } => {
                format!("Create and verify the separately requested {label} output.")
            }
            Self::ExternalResearch => "Independently research current primary or official web sources for the separately requested public research.".to_string(),
            Self::CrossSurface(requirement) => requirement.planner_clause(),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct FileEvidence {
    pub(super) path: String,
    pub(super) start: usize,
    pub(super) end: usize,
}

#[derive(Clone, Debug)]
struct OutputCandidate {
    operation: &'static str,
    path: Option<String>,
    format: Option<String>,
}

pub(super) fn validate_objective_coverage(
    objective: &str,
    draft: &GeneratedActionPlanDraft,
) -> Result<(), PlanCoverageDeficit> {
    decision_pack_contract::validate(objective, draft)?;
    release_recovery_contract::validate(objective, draft)?;
    evidence_artifact_contract::validate(objective, draft)?;
    project_status_contract::validate(objective, draft)?;
    let missing = uncovered_requirement_labels(objective, draft);
    if missing.is_empty() {
        Ok(())
    } else {
        Err(PlanCoverageDeficit::missing(missing))
    }
}

fn uncovered_requirements(objective: &str, draft: &GeneratedActionPlanDraft) -> Vec<Requirement> {
    let requirements = objective_requirements(objective);
    if requirements.is_empty() {
        return Vec::new();
    }

    let output_candidates = output_candidates(draft);
    let mut consumed_outputs = HashSet::new();
    let mut consumed_cross_surface = HashSet::new();
    let mut missing = Vec::new();
    for requirement in requirements {
        let covered = match requirement {
            Requirement::InputFile { ref path } => {
                draft.steps.iter().any(|step| {
                    matches!(&step.tool, GeneratedToolDraft::FileRead { path: actual }
                    if requested_path_matches(path, actual))
                }) || decision_pack_input_paths(draft)
                    .any(|actual| requested_path_matches(path, actual))
                    || release_recovery_input_paths(draft)
                        .any(|actual| requested_path_matches(path, actual))
                    || evidence_artifact_input_paths(draft)
                        .any(|actual| requested_path_matches(path, actual))
            }
            Requirement::InputDirectory { ref path } => draft.steps.iter().any(|step| {
                matches!(&step.tool, GeneratedToolDraft::FileList { path: actual }
                    if requested_path_matches(path, actual))
            }),
            Requirement::OutputFile {
                ref path,
                ref format,
            } => find_output_candidate(&output_candidates, &consumed_outputs, format, Some(path))
                .map(|index| consumed_outputs.insert(index))
                .unwrap_or(false),
            Requirement::OutputFormat { ref format, .. } => {
                find_output_candidate(&output_candidates, &consumed_outputs, format, None)
                    .map(|index| consumed_outputs.insert(index))
                    .unwrap_or(false)
            }
            Requirement::ExternalResearch => requests_external_web_access(draft),
            Requirement::CrossSurface(ref requirement) => {
                compound_requirements::covered(requirement, draft, &mut consumed_cross_surface)
            }
        };
        if !covered {
            missing.push(requirement);
        }
    }
    missing
}

fn uncovered_requirement_labels(objective: &str, draft: &GeneratedActionPlanDraft) -> Vec<String> {
    uncovered_requirements(objective, draft)
        .iter()
        .map(Requirement::label)
        .collect()
}

pub(super) fn compile_decision_pack(
    objective: &str,
    trusted_output_directory: Option<&str>,
) -> Result<Option<GeneratedActionPlanDraft>, PlanCoverageDeficit> {
    decision_pack_contract::compile(objective, trusted_output_directory)
}

pub(super) fn compile_release_recovery(
    objective: &str,
) -> Result<Option<GeneratedActionPlanDraft>, PlanCoverageDeficit> {
    release_recovery_contract::compile(objective)
}

pub(super) fn requests_evidence_bound_decision_pack(objective: &str) -> bool {
    decision_pack_contract::requests_evidence_bound_decision_pack(objective)
}

pub(super) fn resolve_and_compile_decision_pack(
    objective: String,
    resolved_paths: Option<&super::contextual_route::ResolvedContextualObjectivePaths>,
    debug_mode: bool,
) -> Result<(String, Option<GeneratedActionPlanDraft>), super::AgenticLoopError> {
    let objective = resolved_paths.map_or(objective, |resolution| resolution.objective.clone());
    let draft = project_status_contract::compile(&objective)
        .map_or_else(
            || evidence_artifact_contract::compile(&objective),
            |draft| Ok(Some(draft)),
        )
        .and_then(|draft| match draft {
            Some(draft) => Ok(Some(draft)),
            None => compile_release_recovery(&objective),
        })
        .and_then(|draft| match draft {
            Some(draft) => Ok(Some(draft)),
            None => compile_decision_pack(
                &objective,
                resolved_paths.map(|resolution| resolution.output_directory.as_str()),
            ),
        })
        .map_err(|deficit| super::AgenticLoopError {
            code: deficit.code(),
            boundary: "AgentPlanning",
            message: deficit.message(),
            mlc_path: None,
        })?;
    if debug_mode {
        if let Some(draft) = &draft {
            eprintln!(
                "DETERMINISTIC_WORKFLOW_PLAN_COMPILED boundary=AgentPlanning steps={} source=deterministic",
                draft.steps.len()
            );
        }
    }
    Ok((objective, draft))
}

pub(super) fn deterministic_draft_requires_dynamic_route(objective: &str) -> bool {
    evidence_artifact_contract::requests(objective)
}

pub(super) fn deterministic_draft_needs_model_composition(
    objective: &str,
    draft: &GeneratedActionPlanDraft,
) -> bool {
    specialist_composition::needs_model_composition(objective, draft)
}

pub(super) fn compiled_draft_requires_dynamic_route(
    objective: &str,
    draft: &GeneratedActionPlanDraft,
) -> bool {
    deterministic_draft_requires_dynamic_route(objective)
        || deterministic_draft_needs_model_composition(objective, draft)
}

pub(super) fn deterministic_draft_skips_dynamic_route(
    objective: &str,
    draft: Option<&GeneratedActionPlanDraft>,
) -> bool {
    draft.is_some_and(|draft| !compiled_draft_requires_dynamic_route(objective, draft))
}

pub(super) fn deterministic_draft_composition_objective(
    objective: &str,
    draft: &GeneratedActionPlanDraft,
) -> Option<String> {
    specialist_composition::validation_objective(objective, draft)
}

pub(super) fn compose_deterministic_draft(
    objective: &str,
    base: GeneratedActionPlanDraft,
    candidate: GeneratedActionPlanDraft,
) -> Result<GeneratedActionPlanDraft, super::AgenticLoopError> {
    specialist_composition::compose(objective, base, candidate).map_err(|deficit| {
        super::AgenticLoopError {
            code: deficit.code(),
            boundary: "AgentPlanning",
            message: deficit.message(),
            mlc_path: None,
        }
    })
}

pub(super) async fn compose_deterministic_draft_if_needed(
    objective: &str,
    base: GeneratedActionPlanDraft,
    planning_sections: super::PlannerPromptSections,
    service: crate::gemma::GemmaService,
    planner_target: super::PlannerExecutionTarget,
) -> Result<(GeneratedActionPlanDraft, super::PlannerExecutionTarget), super::AgenticLoopError> {
    let Some(composition_objective) = deterministic_draft_composition_objective(objective, &base)
    else {
        return Ok((base, planner_target));
    };
    let (candidate, planner_target) = super::generate_composition_plan_draft(
        composition_objective,
        planning_sections,
        service,
        planner_target,
    )
    .await?;
    Ok((
        compose_deterministic_draft(objective, base, candidate)?,
        planner_target,
    ))
}

pub(super) async fn select_compiled_or_planned_draft(
    objective: &str,
    deterministic: Option<GeneratedActionPlanDraft>,
    contextual: Option<crate::db::ContextualFileActionPreparation>,
    planning_sections: super::PlannerPromptSections,
    service: crate::gemma::GemmaService,
    planner_target: super::PlannerExecutionTarget,
) -> Result<(GeneratedActionPlanDraft, super::PlannerExecutionTarget), super::AgenticLoopError> {
    match (deterministic, contextual) {
        (Some(base), _) => {
            compose_deterministic_draft_if_needed(
                objective,
                base,
                planning_sections,
                service,
                planner_target,
            )
            .await
        }
        (None, Some(crate::db::ContextualFileActionPreparation::Ready(preparation))) => Ok((
            super::contextual_route::deterministic_contextual_file_draft(preparation),
            planner_target,
        )),
        (None, _) => {
            super::generate_plan_draft(
                objective.to_string(),
                planning_sections,
                service,
                planner_target,
            )
            .await
        }
    }
}

pub(super) fn prepare_draft_for_execution(
    objective: &str,
    draft: GeneratedActionPlanDraft,
    web_search_enabled: bool,
) -> Result<GeneratedActionPlanDraft, super::AgenticLoopError> {
    let draft = super::normalize_web_search_plan_draft(objective, draft, web_search_enabled);
    let draft = crate::gemma::normalize_generated_plan_for_known_objectives(objective, draft);
    super::validate_planner_draft_for_execution(objective, &draft, web_search_enabled)?;
    Ok(draft)
}

pub(super) fn validate_connected_service_bindings(
    objective: &str,
    draft: &GeneratedActionPlanDraft,
    persistence: &crate::db::PersistenceEngine,
    project_id: Option<&str>,
) -> Result<(), super::AgenticLoopError> {
    let missing =
        compound_requirements::account_binding_deficits(objective, draft, persistence, project_id);
    if missing.is_empty() {
        return Ok(());
    }
    let deficit = PlanCoverageDeficit::missing(missing);
    Err(super::AgenticLoopError {
        code: "planner_connector_binding_mismatch",
        boundary: "AgentPlanning",
        message: deficit.message(),
        mlc_path: None,
    })
}

pub(super) fn requests_external_web_access(draft: &GeneratedActionPlanDraft) -> bool {
    draft.steps.iter().any(|step| {
        matches!(
            &step.tool,
            GeneratedToolDraft::SovereignDuckDuckGoSearch { .. }
                | GeneratedToolDraft::WebFetch { .. }
        ) || matches!(
            &step.tool,
            GeneratedToolDraft::RegisteredTaskTool { operation, .. }
                if normalized_operation(operation)
                    == crate::tools::evidence_artifacts::COMPARISON_OPERATION
        )
    }) || decision_pack_has_research(draft)
}

pub(super) fn independent_public_searches_only(
    objective: &str,
    draft: &GeneratedActionPlanDraft,
) -> bool {
    let searches = draft
        .steps
        .iter()
        .filter_map(|step| match &step.tool {
            GeneratedToolDraft::SovereignDuckDuckGoSearch { query, .. } => Some(query.as_str()),
            _ => None,
        })
        .chain(decision_pack_research_queries(draft))
        .collect::<Vec<_>>();
    let has_legacy_or_generic = !searches.is_empty();
    let has_structured = decision_pack_has_structured_research(draft);
    (has_legacy_or_generic || has_structured)
        && (!has_legacy_or_generic || bounded_public_queries(objective, searches.into_iter()))
        && (!has_structured || decision_pack_structured_research_valid(objective, draft))
}

pub(super) fn private_app_search_mix_is_unbounded(
    objective: &str,
    draft: &GeneratedActionPlanDraft,
) -> bool {
    crate::local_app_intent::has_private_app_data_intent(objective)
        && !independent_public_searches_only(objective, draft)
}

pub(super) fn signed_plan_independent_public_searches_only(plan: &super::ActionPlan) -> bool {
    let searches = plan
        .steps
        .iter()
        .filter_map(|step| match &step.tool {
            super::Tool::SovereignDuckDuckGoSearch { query, .. } => Some(query.as_str()),
            _ => None,
        })
        .chain(plan.steps.iter().flat_map(|step| {
            match &step.tool {
                super::Tool::RegisteredTaskTool(request)
                    if normalized_operation(&request.operation) == "create_decision_pack" =>
                {
                    request
                        .arguments
                        .get("researchQueries")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                }
                _ => Vec::new(),
            }
        }))
        .collect::<Vec<_>>();
    let has_legacy_or_generic = !searches.is_empty();
    let has_structured = plan.steps.iter().any(|step| match &step.tool {
        super::Tool::RegisteredTaskTool(request)
            if normalized_operation(&request.operation) == "create_decision_pack" =>
        {
            request.arguments.get("researchPolicy").is_some()
        }
        _ => false,
    });
    let structured_valid = plan.steps.iter().any(|step| match &step.tool {
        super::Tool::RegisteredTaskTool(request)
            if normalized_operation(&request.operation) == "create_decision_pack" =>
        {
            request
                .arguments
                .get("researchPolicy")
                .and_then(|value| {
                    serde_json::from_value::<crate::decision_research_policy::ResearchPolicy>(
                        value.clone(),
                    )
                    .ok()
                })
                .is_some_and(|policy| {
                    crate::decision_research_policy::policy_matches_objective(
                        &plan.objective,
                        &policy,
                    )
                    .is_ok()
                })
        }
        _ => false,
    });
    (has_legacy_or_generic || has_structured)
        && (!has_legacy_or_generic || bounded_public_queries(&plan.objective, searches.into_iter()))
        && (!has_structured || structured_valid)
}

fn bounded_public_queries<'a>(objective: &str, searches: impl Iterator<Item = &'a str>) -> bool {
    let mut saw_search = false;
    for query in searches {
        saw_search = true;
        if !crate::sovereign_search::independent_public_research_query_allowed(objective, query) {
            return false;
        }
    }
    saw_search
}

fn decision_pack_input_paths(draft: &GeneratedActionPlanDraft) -> impl Iterator<Item = &str> {
    registered_decision_pack_strings(draft, "inputPaths")
}

fn release_recovery_input_paths(draft: &GeneratedActionPlanDraft) -> impl Iterator<Item = &str> {
    draft.steps.iter().filter_map(|step| match &step.tool {
        GeneratedToolDraft::RegisteredTaskTool {
            operation,
            arguments,
        } if normalized_operation(operation) == "prepare_release_recovery_agenda" => {
            arguments.get("inputPath").and_then(Value::as_str)
        }
        _ => None,
    })
}

fn evidence_artifact_input_paths(draft: &GeneratedActionPlanDraft) -> impl Iterator<Item = &str> {
    draft.steps.iter().filter_map(|step| match &step.tool {
        GeneratedToolDraft::RegisteredTaskTool {
            operation,
            arguments,
        } if normalized_operation(operation)
            == crate::tools::evidence_artifacts::RECOVERY_OPERATION =>
        {
            arguments.get("inputPath").and_then(Value::as_str)
        }
        _ => None,
    })
}

fn decision_pack_research_queries(draft: &GeneratedActionPlanDraft) -> impl Iterator<Item = &str> {
    registered_decision_pack_strings(draft, "researchQueries")
}

fn decision_pack_has_research(draft: &GeneratedActionPlanDraft) -> bool {
    decision_pack_research_queries(draft).next().is_some()
        || decision_pack_has_structured_research(draft)
}

fn decision_pack_has_structured_research(draft: &GeneratedActionPlanDraft) -> bool {
    draft.steps.iter().any(|step| match &step.tool {
        GeneratedToolDraft::RegisteredTaskTool {
            operation,
            arguments,
        } if normalized_operation(operation) == "create_decision_pack" => {
            arguments.get("researchPolicy").is_some()
        }
        _ => false,
    })
}

fn decision_pack_structured_research_valid(
    objective: &str,
    draft: &GeneratedActionPlanDraft,
) -> bool {
    draft.steps.iter().any(|step| match &step.tool {
        GeneratedToolDraft::RegisteredTaskTool {
            operation,
            arguments,
        } if normalized_operation(operation) == "create_decision_pack" => arguments
            .get("researchPolicy")
            .and_then(|value| {
                serde_json::from_value::<crate::decision_research_policy::ResearchPolicy>(
                    value.clone(),
                )
                .ok()
            })
            .is_some_and(|policy| {
                crate::decision_research_policy::policy_matches_objective(objective, &policy)
                    .is_ok()
            }),
        _ => false,
    })
}

fn registered_decision_pack_strings<'a>(
    draft: &'a GeneratedActionPlanDraft,
    field: &'a str,
) -> impl Iterator<Item = &'a str> {
    draft.steps.iter().flat_map(move |step| match &step.tool {
        GeneratedToolDraft::RegisteredTaskTool {
            operation,
            arguments,
        } if normalized_operation(operation) == "create_decision_pack" => arguments
            .get(field)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    })
}

fn objective_requirements(objective: &str) -> Vec<Requirement> {
    let lowered = objective.to_ascii_lowercase();
    let output_directory = explicit_output_directory(objective);
    let mut requirements = Vec::new();
    let mut named_output_formats = HashSet::new();
    let mut seen = HashSet::new();

    for evidence in file_evidence(objective) {
        let Some(format) = file_format(&evidence.path) else {
            continue;
        };
        match file_role(&lowered, evidence.start, evidence.end) {
            Some(FileRole::Input) => {
                let path = normalize_path(&evidence.path);
                if seen.insert(format!("input:{path}")) {
                    requirements.push(Requirement::InputFile { path });
                }
            }
            Some(FileRole::Output) => {
                let raw_path = normalize_path(&evidence.path);
                let path = match (&output_directory, raw_path.contains('/')) {
                    (Some(directory), false) => {
                        format!("{}/{}", directory.trim_end_matches('/'), raw_path)
                    }
                    _ => raw_path,
                };
                if seen.insert(format!("output:{path}")) {
                    named_output_formats.insert(format.clone());
                    requirements.push(Requirement::OutputFile { path, format });
                }
            }
            None => {}
        }
    }

    for evidence in directory_evidence(objective) {
        if file_role(&lowered, evidence.start, evidence.end) != Some(FileRole::Input) {
            continue;
        }
        let path = normalize_path(&evidence.path);
        if seen.insert(format!("directory:{path}")) {
            requirements.push(Requirement::InputDirectory { path });
        }
    }

    for (label, format) in explicit_output_formats(&lowered) {
        if !named_output_formats.contains(&format) && seen.insert(format!("format:{format}")) {
            requirements.push(Requirement::OutputFormat { label, format });
        }
    }
    if crate::sovereign_search::explicit_external_search_requested(objective) {
        requirements.push(Requirement::ExternalResearch);
    }
    requirements.extend(
        compound_requirements::explicit_requirements(objective)
            .into_iter()
            .map(Requirement::CrossSurface),
    );
    requirements
}

pub(super) fn objective_input_file_references(objective: &str) -> Vec<FileEvidence> {
    let lowered = objective.to_ascii_lowercase();
    file_evidence(objective)
        .into_iter()
        .filter(|evidence| {
            file_role(&lowered, evidence.start, evidence.end) == Some(FileRole::Input)
        })
        .collect()
}

pub(super) fn objective_input_directory_references(objective: &str) -> Vec<FileEvidence> {
    let lowered = objective.to_ascii_lowercase();
    directory_evidence(objective)
        .into_iter()
        .filter(|evidence| {
            file_role(&lowered, evidence.start, evidence.end) == Some(FileRole::Input)
        })
        .collect()
}

pub(super) fn objective_output_file_references(objective: &str) -> Vec<FileEvidence> {
    let lowered = objective.to_ascii_lowercase();
    file_evidence(objective)
        .into_iter()
        .filter(|evidence| {
            file_role(&lowered, evidence.start, evidence.end) == Some(FileRole::Output)
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FileRole {
    Input,
    Output,
}

fn file_role(lowered: &str, start: usize, end: usize) -> Option<FileRole> {
    let (clause_start, clause_end) = clause_bounds(lowered, start, end);
    let before = &lowered[clause_start..start];
    let after = &lowered[end..clause_end];
    let clause = &lowered[clause_start..clause_end];
    if non_directive_question(clause) {
        return None;
    }
    if let Some(role) = transformation_role(before) {
        return role;
    }
    if let Some(found) = input_connector_regex().find_iter(before).last() {
        let latest_cue = last_role_cue(before);
        if latest_cue.is_none_or(|(position, _)| found.start() > position) {
            return Some(FileRole::Input);
        }
    }
    let input = last_cue(before, input_cue_regex());
    let output = last_cue(before, output_cue_regex());
    let selected = match (input, output) {
        (Some(input), Some(output)) if input > output => Some((input, FileRole::Input)),
        (Some(_), Some(output)) => Some((output, FileRole::Output)),
        (Some(input), None) => Some((input, FileRole::Input)),
        (None, Some(output)) => Some((output, FileRole::Output)),
        _ => None,
    };
    selected
        .and_then(|(position, role)| (!action_is_negated(before, position)).then_some(role))
        .or_else(|| {
            trailing_input_origin_regex()
                .is_match(after)
                .then_some(FileRole::Input)
        })
}

fn clause_bounds(value: &str, start: usize, end: usize) -> (usize, usize) {
    let punctuation_start = value[..start]
        .rfind(['!', '?', ';'])
        .map(|index| index + 1)
        .unwrap_or(0);
    let paragraph_start = value[..start]
        .rfind("\n\n")
        .map(|index| index + 2)
        .unwrap_or(0);
    let sentence_start = [
        value[..start].rfind(". ").map(|index| index + 2),
        value[..start].rfind(".\n").map(|index| index + 2),
        value[..start].rfind(".\r\n").map(|index| index + 3),
    ]
    .into_iter()
    .flatten()
    .max()
    .unwrap_or(0);
    let punctuation_end = value[end..]
        .find(['!', '?', ';'])
        .map(|index| end + index)
        .unwrap_or(value.len());
    let paragraph_end = value[end..]
        .find("\n\n")
        .map(|index| end + index)
        .unwrap_or(value.len());
    let sentence_end = [
        value[end..].find(". ").map(|index| end + index),
        value[end..].find(".\n").map(|index| end + index),
        value[end..].find(".\r\n").map(|index| end + index),
    ]
    .into_iter()
    .flatten()
    .min()
    .unwrap_or(value.len());
    (
        punctuation_start.max(paragraph_start).max(sentence_start),
        punctuation_end.min(paragraph_end).min(sentence_end),
    )
}

fn last_role_cue(value: &str) -> Option<(usize, FileRole)> {
    let input = last_cue(value, input_cue_regex()).map(|position| (position, FileRole::Input));
    let output = last_cue(value, output_cue_regex()).map(|position| (position, FileRole::Output));
    [input, output]
        .into_iter()
        .flatten()
        .max_by_key(|item| item.0)
}

fn transformation_role(before: &str) -> Option<Option<FileRole>> {
    let action = transformation_cue_regex().find_iter(before).last()?;
    if action_is_negated(before, action.start()) {
        return Some(None);
    }
    let target = transformation_target_regex().find_iter(before).last();
    Some(Some(
        if target.is_some_and(|found| found.start() > action.start()) {
            FileRole::Output
        } else {
            FileRole::Input
        },
    ))
}

fn last_cue(value: &str, regex: &Regex) -> Option<usize> {
    regex.find_iter(value).last().map(|found| found.start())
}

fn input_cue_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?i)\b(?:read|list|inspect|review|analy[sz]e|reconcile|load|open|ingest|import|input|source|using)\b")
            .expect("input cue regex")
    })
}

fn trailing_input_origin_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r"(?i)^\s*from\s+(?:my\s+|the\s+|this\s+)?[a-z0-9][a-z0-9 _-]{0,120}\s+(?:folder|directory)\b",
        )
        .expect("trailing input origin regex")
    })
}

fn output_cue_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?i)\b(?:create|deliver|write|save|generate|produce|export|output|make|build|render|edit|modify|update)\b")
            .expect("output cue regex")
    })
}

fn input_connector_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r#"(?i)\b(?:using|from|with|based\s+on|sourced\s+from)\s*[`'"]?\s*$"#)
            .expect("input connector regex")
    })
}

fn transformation_cue_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?i)\b(?:convert|transform|summari[sz]e)\b").expect("transformation cue regex")
    })
}

fn transformation_target_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r#"(?i)\b(?:to|into|as)\s*[`'"]?\s*$"#).expect("target connector regex")
    })
}

fn file_evidence(objective: &str) -> Vec<FileEvidence> {
    let mut evidence = delimited_file_evidence(objective);
    for found in bare_absolute_file_evidence(objective) {
        if evidence
            .iter()
            .any(|existing| found.start >= existing.start && found.end <= existing.end)
        {
            continue;
        }
        evidence.push(found);
    }
    for found in plain_file_regex().find_iter(objective) {
        if evidence
            .iter()
            .any(|existing| found.start() >= existing.start && found.end() <= existing.end)
        {
            continue;
        }
        let prefix = trailing_characters(&objective[..found.start()], 12);
        if prefix.contains("://") || prefix.ends_with('@') {
            continue;
        }
        let path = normalize_path(found.as_str());
        if file_format(&path).is_some() {
            evidence.push(FileEvidence {
                path,
                start: found.start(),
                end: found.end(),
            });
        }
    }
    evidence.sort_by_key(|item| item.start);
    evidence
}

fn directory_evidence(objective: &str) -> Vec<FileEvidence> {
    let mut evidence = Vec::new();
    let files = file_evidence(objective);
    for captures in backtick_regex().captures_iter(objective) {
        let Some(found) = captures.get(1) else {
            continue;
        };
        let path = found.as_str().trim();
        let following = objective[found.end()..].trim_start().to_ascii_lowercase();
        if !path.contains("://")
            && file_format(path).is_none()
            && (path.contains('/')
                || following.starts_with("folder")
                || following.starts_with("directory"))
        {
            evidence.push(FileEvidence {
                path: normalize_path(path),
                start: found.start(),
                end: found.end(),
            });
        }
    }
    for found in bare_absolute_directory_evidence(objective) {
        if files
            .iter()
            .any(|file| found.start >= file.start && found.end <= file.end)
            || evidence
                .iter()
                .any(|existing| found.start >= existing.start && found.end <= existing.end)
        {
            continue;
        }
        evidence.push(found);
    }
    for found in plain_directory_regex().find_iter(objective) {
        let candidate = found.as_str().trim_end_matches('.');
        let candidate_end = found.start() + candidate.len();
        if files
            .iter()
            .any(|file| found.start() >= file.start && candidate_end <= file.end)
            || evidence
                .iter()
                .any(|existing| found.start() >= existing.start && candidate_end <= existing.end)
            || file_format(candidate).is_some()
        {
            continue;
        }
        let prefix = trailing_characters(&objective[..found.start()], 12);
        if prefix.contains("://") || prefix.ends_with('@') {
            continue;
        }
        evidence.push(FileEvidence {
            path: normalize_path(candidate),
            start: found.start(),
            end: candidate_end,
        });
    }
    evidence.sort_by_key(|item| item.start);
    evidence
}

fn trailing_characters(value: &str, count: usize) -> &str {
    let start = value
        .char_indices()
        .rev()
        .nth(count.saturating_sub(1))
        .map(|(index, _)| index)
        .unwrap_or(0);
    &value[start..]
}

fn delimited_file_evidence(objective: &str) -> Vec<FileEvidence> {
    let mut result = Vec::new();
    for captures in backtick_regex().captures_iter(objective) {
        let Some(found) = captures.get(1) else {
            continue;
        };
        let path = found.as_str().trim();
        if !path.contains("://") && file_format(path).is_some() {
            result.push(FileEvidence {
                path: normalize_path(path),
                start: found.start(),
                end: found.end(),
            });
        }
    }
    result
}

fn backtick_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"`([^`\r\n]{1,4096})`").expect("backtick regex"))
}

fn plain_file_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?i)(?:~?/|\.{1,2}/)?(?:[a-z0-9_~.-]+/)*[a-z0-9_-]+\.[a-z0-9]{1,10}")
            .expect("file reference regex")
    })
}

fn absolute_path_start_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r"(?i)(?:^|[\s(\[{:;,])(/(?:applications|library|system|users|volumes|bin|dev|etc|home|opt|private|sbin|tmp|usr|var)(?:/|\\ ))",
        )
        .expect("absolute path start regex")
    })
}

fn file_extension_boundary_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(concat!(
            r"(?i)(\.(?:c|cpp|csv|db|doc|docx|gif|go|gz|h|hpp|htm|html|java|jpeg|jpg|js|json|jsx|kt|md|markdown|pdf|png|ppt|pptx|py|rb|rs|rtf|sh|sql|sqlite|svg|swift|tar|toml|ts|tsv|tsx|txt|webp|xls|xlsx|xml|yaml|yml|zip|zsh))(?:$|[.,:;!?\])}]|\s+(?:and|or|then|from|with|without|to|in|into|inside|under|at|for|before|after|while|but|please|containing|",
            r"contains|analy[sz]e|archive|attach|compare|copy|create|delete|describe|draft|email|explain|export|import|inspect|list|move|open|prepare|publish|read|recommend|remove|rename|review|run|save|send|share|show|summari[sz]e|trash|upload|write)\b)",
        ))
        .expect("file extension boundary regex")
    })
}

fn absolute_directory_clause_boundary_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r"(?i)(?:\s+(?:in|from)\s+(?:my\s+|the\s+|this\s+)?(?:testing|project|source|input)\s+(?:folder|directory)\b|\s+(?:and\s+then|then|and|or|but)\s+(?:analy[sz]e|archive|attach|compare|copy|create|delete|describe|draft|email|explain|export|import|inspect|list|move|open|prepare|publish|read|recommend|remove|rename|review|run|save|send|share|show|summari[sz]e|trash|upload|write)\b|[.!?;](?:\s|$))",
        )
        .expect("absolute directory clause boundary regex")
    })
}

fn bare_absolute_file_evidence(objective: &str) -> Vec<FileEvidence> {
    absolute_path_start_regex()
        .captures_iter(objective)
        .filter_map(|captures| {
            let start = captures.get(1)?.start();
            let remainder = &objective[start..];
            let clause_end = absolute_directory_clause_boundary_regex()
                .find(remainder)
                .map(|boundary| boundary.start())
                .unwrap_or(remainder.len());
            let bounded_remainder = &remainder[..clause_end];
            let extension = file_extension_boundary_regex()
                .captures(bounded_remainder)?
                .get(1)?;
            let end = start + extension.end();
            let path = normalize_path(&objective[start..end]);
            file_format(&path).map(|_| FileEvidence { path, start, end })
        })
        .collect()
}

fn bare_absolute_directory_evidence(objective: &str) -> Vec<FileEvidence> {
    absolute_path_start_regex()
        .captures_iter(objective)
        .filter_map(|captures| {
            let start = captures.get(1)?.start();
            let remainder = &objective[start..];
            let raw_end = absolute_directory_clause_boundary_regex()
                .find(remainder)
                .map(|boundary| boundary.start())
                .unwrap_or(remainder.len());
            let bounded_remainder = &remainder[..raw_end];
            if file_extension_boundary_regex()
                .captures(bounded_remainder)
                .is_some()
            {
                return None;
            }
            let raw = remainder[..raw_end].trim_end_matches(|character: char| {
                character.is_whitespace() || matches!(character, '.' | ',' | ':' | ';' | '!' | '?')
            });
            let path = normalize_path(raw);
            (!path.is_empty() && Path::new(&path).is_absolute() && file_format(&path).is_none())
                .then(|| FileEvidence {
                    path,
                    start,
                    end: start + raw.len(),
                })
        })
        .collect()
}

fn plain_directory_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r"(?i)(?:(?:~|\.{1,2})?/(?:[a-z0-9_~.-]+(?:[ ][a-z0-9_~.-]+)*/)+[a-z0-9_~.-]+|(?:~|\.{1,2})?/(?:[a-z0-9_~.-]+|\\ )+(?:/(?:[a-z0-9_~.-]+|\\ )+)+)",
        )
        .expect("directory path regex")
    })
}

pub(super) fn explicit_output_directory(objective: &str) -> Option<String> {
    let captures = output_directory_regex().captures(objective)?;
    let value = captures.get(1).or_else(|| captures.get(2))?.as_str();
    let normalized = normalize_path(value);
    (!normalized.is_empty()).then_some(normalized)
}

fn output_directory_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?:(?:create|make)\s+(?:a\s+)?(?:new\s+)?`?([a-z0-9_~./-]+)`?\s+(?:folder|directory)\b|(?:create|make)\s+(?:a\s+)?(?:new\s+)?(?:folder|directory)\s+(?:named|called)\s+`?([a-z0-9_~./-]+)`?\b)",
        )
        .expect("output directory regex")
    })
}

fn explicit_output_formats(lowered: &str) -> Vec<(String, String)> {
    let mut result = Vec::new();
    for segment in lowered
        .split(['!', '?', ';', '\n'])
        .flat_map(|segment| segment.split(". "))
    {
        let without_delimited_files = backtick_regex().replace_all(segment, " ");
        let segment = plain_file_regex().replace_all(&without_delimited_files, " ");
        if !positive_action_segment(
            &segment,
            &[
                "create", "deliver", "write", "save", "generate", "produce", "export", "make",
                "build", "render",
            ],
        ) {
            continue;
        }
        for (label, format, terms) in [
            (
                "spreadsheet",
                "xlsx",
                &["spreadsheet", "workbook", "excel"] as &[&str],
            ),
            (
                "presentation",
                "pptx",
                &["presentation", "slide deck", "powerpoint"],
            ),
            ("PDF", "pdf", &["pdf"]),
            ("Markdown", "md", &["markdown"]),
            ("Word document", "docx", &["word document"]),
            ("CSV", "csv", &["csv"]),
            ("JSON", "json", &["json"]),
            ("text file", "txt", &["text file"]),
            ("rich text", "rtf", &["rich text", "rtf"]),
            ("HTML", "html", &["html"]),
            ("XML", "xml", &["xml"]),
        ] {
            if terms.iter().any(|term| contains_term(&segment, term)) {
                result.push((label.to_string(), format.to_string()));
            }
        }
    }
    result
}

fn contains_term(value: &str, term: &str) -> bool {
    value.match_indices(term).any(|(index, _)| {
        let before = value[..index].chars().next_back();
        let after = value[index + term.len()..].chars().next();
        before.is_none_or(|character| !character.is_ascii_alphanumeric())
            && after.is_none_or(|character| !character.is_ascii_alphanumeric())
    })
}

fn positive_action_segment(segment: &str, actions: &[&str]) -> bool {
    !non_directive_question(segment)
        && actions.iter().any(|action| {
            term_positions(segment, action).any(|position| !action_is_negated(segment, position))
        })
}

fn term_positions<'a>(value: &'a str, term: &'a str) -> impl Iterator<Item = usize> + 'a {
    value.match_indices(term).filter_map(move |(index, _)| {
        let before = value[..index].chars().next_back();
        let after = value[index + term.len()..].chars().next();
        (before.is_none_or(|character| !character.is_ascii_alphanumeric())
            && after.is_none_or(|character| !character.is_ascii_alphanumeric()))
        .then_some(index)
    })
}

fn action_is_negated(value: &str, action_index: usize) -> bool {
    let prefix = &value[..action_index];
    let reset = [" but ", " however ", " instead "]
        .iter()
        .filter_map(|marker| prefix.rfind(marker).map(|index| index + marker.len()))
        .max()
        .unwrap_or(0);
    let scope = &prefix[reset..];
    if scope.trim_end().ends_with("not only") {
        return false;
    }
    scope
        .split(|character: char| !character.is_alphabetic() && character != '\'')
        .filter(|token| !token.is_empty())
        .rev()
        .take(6)
        .any(|token| {
            matches!(
                token,
                "not" | "no" | "never" | "without" | "avoid" | "avoiding" | "don't" | "dont"
            )
        })
}

fn non_directive_question(segment: &str) -> bool {
    let value = segment.trim().trim_start_matches([',', ':']).trim();
    if ["can you ", "could you ", "would you ", "will you "]
        .iter()
        .any(|prefix| value.starts_with(prefix))
    {
        return false;
    }
    [
        "how ",
        "why ",
        "what ",
        "when ",
        "where ",
        "who ",
        "whether ",
        "should i ",
        "can i ",
        "could i ",
        "would i ",
        "did you ",
        "do you ",
        "does ",
        "is ",
        "are ",
        "was ",
        "were ",
        "explain ",
        "help me understand ",
        "tell me how ",
        "tell me whether ",
    ]
    .iter()
    .any(|prefix| value.starts_with(prefix))
}

fn output_candidates(draft: &GeneratedActionPlanDraft) -> Vec<OutputCandidate> {
    draft
        .steps
        .iter()
        .flat_map(|step| match &step.tool {
            GeneratedToolDraft::FileWrite { path, .. } => file_format(path)
                .filter(|format| TEXT_FILE_FORMATS.contains(&format.as_str()))
                .map(|format| OutputCandidate {
                    operation: "file_write",
                    path: Some(normalize_path(path)),
                    format: Some(format),
                })
                .into_iter()
                .collect::<Vec<_>>(),
            GeneratedToolDraft::TelemetryArchive { output_path } => file_format(output_path)
                .filter(|format| matches!(format.as_str(), "gz" | "tar" | "zip"))
                .map(|format| OutputCandidate {
                    operation: "telemetry_archive",
                    path: Some(normalize_path(output_path)),
                    format: Some(format),
                })
                .into_iter()
                .collect::<Vec<_>>(),
            GeneratedToolDraft::RegisteredTaskTool {
                operation,
                arguments,
            } => registered_output_candidates(operation, arguments),
            _ => Vec::new(),
        })
        .collect()
}

fn registered_output_candidates(operation: &str, arguments: &Value) -> Vec<OutputCandidate> {
    match normalized_operation(operation).as_str() {
        "create_file" => {
            let path = arguments
                .pointer("/file/destinationPath")
                .and_then(Value::as_str)
                .map(normalize_path);
            let format = arguments
                .pointer("/file/format")
                .and_then(Value::as_str)
                .map(str::to_ascii_lowercase)
                .or_else(|| path.as_deref().and_then(file_format));
            vec![OutputCandidate {
                operation: "create_file",
                path,
                format,
            }]
        }
        "prepare_release_recovery_agenda" => {
            let path = arguments
                .get("outputPath")
                .and_then(Value::as_str)
                .map(normalize_path);
            vec![OutputCandidate {
                operation: "prepare_release_recovery_agenda",
                format: path.as_deref().and_then(file_format),
                path,
            }]
        }
        crate::tools::evidence_artifacts::COMPARISON_OPERATION
        | crate::tools::evidence_artifacts::RECOVERY_OPERATION => {
            let path = arguments
                .get("outputPath")
                .and_then(Value::as_str)
                .map(normalize_path);
            vec![OutputCandidate {
                operation: crate::tools::evidence_artifacts::COMPARISON_OPERATION,
                format: path.as_deref().and_then(file_format),
                path,
            }]
        }
        "create_spreadsheet" => vec![OutputCandidate {
            operation: "create_spreadsheet",
            path: None,
            format: Some("xlsx".to_string()),
        }],
        "create_presentation" => vec![OutputCandidate {
            operation: "create_presentation",
            path: None,
            format: Some("pptx".to_string()),
        }],
        "create_decision_pack" => {
            let Some(directory) = arguments.get("outputDirectory").and_then(Value::as_str) else {
                return Vec::new();
            };
            [
                ("workbook", "xlsx"),
                ("presentation", "pptx"),
                ("pdf", "pdf"),
                ("sources", "md"),
            ]
            .into_iter()
            .filter_map(|(field, format)| {
                let name = arguments
                    .pointer(&format!("/outputs/{field}"))
                    .and_then(Value::as_str)?;
                Some(OutputCandidate {
                    operation: "create_decision_pack",
                    path: Some(normalize_path(&format!(
                        "{}/{}",
                        directory.trim_end_matches('/'),
                        name
                    ))),
                    format: Some(format.to_string()),
                })
            })
            .collect()
        }
        _ => Vec::new(),
    }
}

fn find_output_candidate(
    candidates: &[OutputCandidate],
    consumed: &HashSet<usize>,
    format: &str,
    requested_path: Option<&str>,
) -> Option<usize> {
    candidates
        .iter()
        .enumerate()
        .filter(|(index, _)| !consumed.contains(index))
        .find(|candidate| {
            candidate_supports_format(candidate.1, format)
                && match (requested_path, candidate.1.path.as_deref()) {
                    (Some(requested), Some(actual)) => requested_path_matches(requested, actual),
                    (Some(_), None) => false,
                    (None, _) => true,
                }
        })
        .map(|(index, _)| index)
}

fn candidate_supports_format(candidate: &OutputCandidate, format: &str) -> bool {
    candidate
        .format
        .as_deref()
        .is_some_and(|actual| actual.eq_ignore_ascii_case(format))
        || (candidate.operation == "create_spreadsheet" && format == "xls")
        || (candidate.operation == "create_presentation" && format == "ppt")
}

fn normalized_operation(operation: &str) -> String {
    operation.trim().replace('-', "_").to_ascii_lowercase()
}

fn requested_path_matches(requested: &str, actual: &str) -> bool {
    let requested = normalize_path(requested);
    let actual = normalize_path(actual);
    // Absolute objectives are exact here. Relative objectives can only prove a
    // component-aligned suffix; Shield binds the concrete approved root later.
    if requested.starts_with('/') {
        return actual == requested;
    }
    let requested = requested.strip_prefix("~/").unwrap_or(&requested);
    actual == requested || actual.ends_with(&format!("/{requested}"))
}

fn normalize_path(value: &str) -> String {
    value
        .trim()
        .trim_matches(['`', '\'', '"'])
        .replace("\\ ", " ")
        .replace("\\~", "~")
        .replace("//", "/")
        .trim_start_matches("./")
        .trim_end_matches('/')
        .to_string()
}

fn file_format(path: &str) -> Option<String> {
    let extension = Path::new(path)
        .extension()
        .and_then(|value| value.to_str())?
        .to_ascii_lowercase();
    FILE_EXTENSIONS
        .contains(&extension.as_str())
        .then_some(extension)
}
#[cfg(test)]
#[path = "tests/plan_coverage.rs"]
mod tests;
