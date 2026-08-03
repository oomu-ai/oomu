use super::*;
use std::{
    collections::{HashSet, VecDeque},
    path::{Component, Path, PathBuf},
};

const MAX_GROUNDING_DEPTH: usize = 4;
const MAX_GROUNDING_ENTRIES: usize = 4_096;

#[derive(Debug, Clone, Eq, PartialEq)]
struct GroundedObjectivePaths {
    inputs: Vec<PathBuf>,
    output_directory: PathBuf,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(super) struct ResolvedContextualObjectivePaths {
    pub(super) objective: String,
    pub(super) output_directory: String,
}

/// Returns the narrowest real directory that can safely ground the path references in an
/// objective without reading any directory contents. This is intentionally metadata-only: the
/// caller must still take the candidate through Shield Gate before resolution scans it.
pub(super) fn approval_candidate_root(objective: &str) -> Option<PathBuf> {
    let mut candidates = plan_coverage::objective_input_file_references(objective)
        .into_iter()
        .chain(plan_coverage::objective_input_directory_references(
            objective,
        ))
        .filter_map(|reference| {
            let requested = PathBuf::from(reference.path);
            if !requested.is_absolute() {
                return None;
            }
            let mut cursor = if is_regular_non_symlink_file(&requested) {
                requested.parent().map(Path::to_path_buf)
            } else {
                Some(requested)
            }?;
            loop {
                if let Ok(directory) = canonical_real_directory(&cursor) {
                    return Some(directory);
                }
                cursor = cursor.parent()?.to_path_buf();
            }
        })
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.dedup();
    if candidates.len() == 1 {
        return candidates.pop();
    }
    if let Some(deepest) = candidates
        .iter()
        .max_by_key(|candidate| candidate.components().count())
        .filter(|deepest| {
            candidates
                .iter()
                .all(|candidate| deepest.starts_with(candidate))
        })
    {
        return Some(deepest.clone());
    }
    let common = candidates.first()?.ancestors().find(|ancestor| {
        ancestor.parent().is_some()
            && candidates
                .iter()
                .all(|candidate| candidate.starts_with(ancestor))
    })?;
    canonical_real_directory(common).ok()
}

pub(super) fn resolve_contextual_objective_paths(
    objective: &str,
    session_id: Option<&str>,
    persistence: &PersistenceEngine,
    identity: &SovereignIdentity,
) -> Result<Option<ResolvedContextualObjectivePaths>, AgenticLoopError> {
    if !plan_coverage::requests_contextual_path_grounding(objective) {
        return Ok(None);
    }
    let references = plan_coverage::objective_input_file_references(objective);
    let output_references = plan_coverage::objective_output_file_references(objective);
    let Some(requested_output) = plan_coverage::explicit_output_directory(objective)
        .or_else(|| common_relative_output_directory(&output_references))
    else {
        return Ok(None);
    };
    let needs_grounding = references.iter().any(|reference| {
        let path = Path::new(&reference.path);
        !path.is_absolute() || !is_regular_non_symlink_file(path)
    }) || !Path::new(&requested_output).is_absolute();
    if !needs_grounding {
        return Ok(None);
    }

    let session_id = session_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(missing_verified_grounding_root)?;
    let verified_directories = persistence
        .verified_filesystem_contexts(session_id, "directory", identity)
        .map_err(|_| verified_grounding_context_unavailable())?
        .into_iter()
        .map(|context| context.canonical_path)
        .collect::<Vec<_>>();
    let project_directories = persistence
        .active_project_source_directories_for_session(session_id)
        .map_err(|_| AgenticLoopError {
            code: "contextual_project_roots_unavailable",
            boundary: "ProjectFolderContext",
            message: "OOMU couldn’t inspect this chat’s approved Project folders. Reopen the Project or approve a read of the intended source folder, then retry. Nothing was changed."
                .to_string(),
            mlc_path: None,
        })?;
    // An output-only relative destination belongs to the active Project when
    // one exists. A stale same-session folder receipt must never pull that
    // output into an unrelated directory. Input-bearing objectives retain the
    // broader lookup because the requested filenames disambiguate the root.
    let mut roots: Vec<&str> = if references.is_empty() && !project_directories.is_empty() {
        project_directories.iter().map(String::as_str).collect()
    } else {
        verified_directories
            .iter()
            .map(String::as_str)
            .chain(project_directories.iter().map(String::as_str))
            .collect()
    };
    roots.sort_unstable();
    roots.dedup();
    let input_names = references
        .iter()
        .map(|reference| {
            Path::new(&reference.path)
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .ok_or_else(|| AgenticLoopError {
                    code: "contextual_input_name_invalid",
                    boundary: "VerifiedFilesystemContext",
                    message: "OOMU couldn’t identify one requested input filename safely. State the filename again and retry. Nothing was changed."
                        .to_string(),
                    mlc_path: None,
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if input_names.iter().collect::<HashSet<_>>().len() != input_names.len() {
        return Err(AgenticLoopError {
            code: "contextual_input_names_ambiguous",
            boundary: "VerifiedFilesystemContext",
            message: "OOMU found duplicate input filenames in the request. Use distinct file paths so each source can be verified. Nothing was changed."
                .to_string(),
            mlc_path: None,
        });
    }
    let output_parent_label = requested_output_parent_label(objective);
    let mut candidates = Vec::new();
    for root in roots {
        if references.is_empty() {
            candidates.extend(grounded_output_candidates(
                Path::new(root),
                &requested_output,
                output_parent_label.as_deref(),
            ));
        } else {
            candidates.extend(grounded_candidates(
                Path::new(root),
                &input_names,
                &requested_output,
                output_parent_label.as_deref(),
            ));
        }
    }
    candidates.sort_by(|left, right| {
        left.inputs
            .cmp(&right.inputs)
            .then(left.output_directory.cmp(&right.output_directory))
    });
    candidates.dedup();
    let grounded = match candidates.len() {
        1 => candidates.remove(0),
        0 => return Err(missing_verified_grounding_root()),
        _ => {
            return Err(AgenticLoopError {
                code: "contextual_grounding_ambiguous",
                boundary: "VerifiedFilesystemContext",
                message: "More than one approved folder contains the requested input filenames. Use one Project folder or approve a read of the exact intended folder, then retry. Nothing was changed."
                    .to_string(),
                mlc_path: None,
            })
        }
    };
    let rewritten = rewrite_objective_with_grounded_paths(
        objective,
        &references,
        &output_references,
        &grounded,
    )?;
    let output_directory = prompt_safe_path(&grounded.output_directory)?;
    Ok(Some(ResolvedContextualObjectivePaths {
        objective: rewritten,
        output_directory,
    }))
}

pub(super) async fn resolve_with_bounded_approval(
    objective: &str,
    request: &AgentObjectiveRequest,
    session_project_id: Option<String>,
    persistence: tauri::State<'_, PersistenceEngine>,
    identity: tauri::State<'_, SovereignIdentity>,
    approvals: tauri::State<'_, ShieldApprovalManager>,
    scope_trust: tauri::State<'_, ScopeTrustManager>,
    leases: tauri::State<'_, ActuationLeaseManager>,
    app: tauri::AppHandle,
) -> Result<Option<ResolvedContextualObjectivePaths>, AgenticLoopError> {
    let resolve = || {
        resolve_contextual_objective_paths(
            objective,
            request.session_id.as_deref(),
            persistence.inner(),
            identity.inner(),
        )
    };
    match resolve() {
        Ok(paths) => Ok(paths),
        Err(error) if error.code == "contextual_grounding_root_required" => {
            let Some(candidate) = approval_candidate_root(objective) else {
                return Err(error);
            };
            let output = crate::shield_gate::execute_native_file_access_command(
                ExecuteCommandRequest {
                    action: RequestedAction {
                        kind: "file_list".to_string(),
                        principal: Some(request.agent_id.clone()),
                        path: Some(candidate.display().to_string()),
                        content: None,
                    },
                    logical_certificate: None,
                    session_id: request.session_id.clone(),
                    turn_id: request.turn_id.clone(),
                    generation_token: request.generation_token.clone(),
                    agent_id: Some(request.agent_id.clone()),
                    provider_id: request.provider_id.clone(),
                    model_id: request.model_id.clone(),
                    parent_turn_id: request.parent_turn_id.clone(),
                    root_turn_id: request.root_turn_id.clone(),
                    turn_kind: request.turn_kind.clone(),
                    project_id: session_project_id,
                    task_run_id: None,
                },
                persistence.clone(),
                identity.clone(),
                approvals,
                scope_trust,
                leases,
                app,
            )
            .await
            .map_err(|shield_error| AgenticLoopError {
                code: if shield_error.code == "shield_approval_denied" {
                    "permission_denied"
                } else {
                    "permission_request_failed"
                },
                boundary: "ShieldApprovalManager",
                message: if shield_error.code == "shield_approval_denied" {
                    "Permission wasn’t granted. Nothing was changed.".to_string()
                } else {
                    "OOMU couldn’t verify the requested folder. Nothing was changed. Try again."
                        .to_string()
                },
                mlc_path: None,
            })?;
            if !output.verified || output.status.as_str() != "completed" {
                return Err(AgenticLoopError {
                    code: "contextual_grounding_read_failed",
                    boundary: "VerifiedFilesystemContext",
                    message:
                        "OOMU couldn’t verify the requested folder. Nothing was changed. Try again."
                            .to_string(),
                    mlc_path: None,
                });
            }
            let candidate_path = candidate.to_string_lossy();
            let receipt_saved = persistence
                .verified_filesystem_contexts(
                    request.session_id.as_deref().unwrap_or_default(),
                    "directory",
                    identity.inner(),
                )
                .map_err(|_| verified_grounding_context_unavailable())?
                .into_iter()
                .any(|context| context.canonical_path == candidate_path);
            if !receipt_saved {
                return Err(AgenticLoopError {
                    code: "contextual_grounding_receipt_failed",
                    boundary: "VerifiedFilesystemContext",
                    message: "OOMU verified the folder but could not save the approval receipt needed to continue. Your files were not changed. Try again."
                        .to_string(),
                    mlc_path: None,
                });
            }
            resolve()
        }
        Err(error) => Err(error),
    }
}

fn grounded_candidates(
    root: &Path,
    input_names: &[String],
    requested_output: &str,
    output_parent_label: Option<&str>,
) -> Vec<GroundedObjectivePaths> {
    let Ok(root) = canonical_real_directory(root) else {
        return Vec::new();
    };
    let mut queue = VecDeque::from([(root.clone(), 0usize)]);
    let mut scanned_entries = 0usize;
    let mut result = Vec::new();
    while let Some((directory, depth)) = queue.pop_front() {
        let inputs = input_names
            .iter()
            .map(|name| canonical_regular_file(&directory.join(name)))
            .collect::<Option<Vec<_>>>();
        if let Some(inputs) = inputs {
            if let Some(output_parent) =
                resolved_output_parent(&directory, &root, output_parent_label)
            {
                if let Some(output_directory) =
                    joined_output_directory(&output_parent, requested_output)
                {
                    result.push(GroundedObjectivePaths {
                        inputs,
                        output_directory,
                    });
                }
            }
        }
        if depth >= MAX_GROUNDING_DEPTH || scanned_entries >= MAX_GROUNDING_ENTRIES {
            continue;
        }
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            scanned_entries += 1;
            if scanned_entries > MAX_GROUNDING_ENTRIES {
                break;
            }
            let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                queue.push_back((entry.path(), depth + 1));
            }
        }
    }
    result
}

fn grounded_output_candidates(
    root: &Path,
    requested_output: &str,
    output_parent_label: Option<&str>,
) -> Vec<GroundedObjectivePaths> {
    let Ok(root) = canonical_real_directory(root) else {
        return Vec::new();
    };
    if output_parent_label.is_some_and(|label| {
        !root
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case(label))
    }) {
        return Vec::new();
    }
    joined_output_directory(&root, requested_output)
        .map(|output_directory| GroundedObjectivePaths {
            inputs: Vec::new(),
            output_directory,
        })
        .into_iter()
        .collect()
}

fn canonical_real_directory(path: &Path) -> Result<PathBuf, ()> {
    let metadata = fs::symlink_metadata(path).map_err(|_| ())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(());
    }
    fs::canonicalize(path).map_err(|_| ())
}

fn canonical_regular_file(path: &Path) -> Option<PathBuf> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return None;
    }
    let canonical = fs::canonicalize(path).ok()?;
    let expected_parent = fs::canonicalize(path.parent()?).ok()?;
    (canonical.parent() == Some(expected_parent.as_path())).then_some(canonical)
}

fn is_regular_non_symlink_file(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
}

fn resolved_output_parent(
    input_directory: &Path,
    trusted_root: &Path,
    label: Option<&str>,
) -> Option<PathBuf> {
    let Some(label) = label else {
        return Some(trusted_root.to_path_buf());
    };
    input_directory
        .ancestors()
        .take(MAX_GROUNDING_DEPTH + 2)
        .find(|candidate| {
            candidate
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case(label))
        })
        .map(Path::to_path_buf)
        .or_else(|| {
            trusted_root
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case(label))
                .then(|| trusted_root.to_path_buf())
        })
}

fn joined_output_directory(parent: &Path, requested: &str) -> Option<PathBuf> {
    let requested = Path::new(requested);
    if requested.is_absolute()
        || requested
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return None;
    }
    Some(parent.join(requested))
}

fn requested_output_parent_label(objective: &str) -> Option<String> {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX
        .get_or_init(|| {
            Regex::new(
                r"(?i)\b(?:folder|directory)(?:\s+(?:named|called)\s+`?[a-z0-9_~./-]+`?)?\s+(?:in|inside)\s+(?:my\s+|the\s+|this\s+)?([a-z0-9][a-z0-9 _-]{0,79}?)\s+(?:folder|directory)\b",
            )
            .expect("contextual output parent regex")
        })
        .captures(objective)
        .and_then(|captures| captures.get(1))
        .map(|capture| capture.as_str().trim().to_string())
        .or_else(|| destination_folder_label(objective))
}

fn destination_folder_label(objective: &str) -> Option<String> {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX
        .get_or_init(|| {
            Regex::new(
                r"(?i)\b(?:in|inside|under|within)\s+(?:my\s+|the\s+|this\s+)?([a-z0-9][a-z0-9 _-]{0,79}?)\s+(?:folder|directory)\b",
            )
            .expect("contextual destination folder regex")
        })
        .captures_iter(objective)
        .last()
        .and_then(|captures| captures.get(1))
        .map(|capture| capture.as_str().trim().to_string())
}

fn common_relative_output_directory(references: &[plan_coverage::FileEvidence]) -> Option<String> {
    let mut parents = references
        .iter()
        .filter_map(|reference| {
            let path = Path::new(&reference.path);
            if path.is_absolute()
                || path
                    .components()
                    .any(|component| !matches!(component, Component::Normal(_)))
            {
                return None;
            }
            path.parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .map(|parent| parent.to_string_lossy().into_owned())
        })
        .collect::<Vec<_>>();
    parents.sort();
    parents.dedup();
    (parents.len() == 1).then(|| parents.remove(0))
}

fn missing_verified_grounding_root() -> AgenticLoopError {
    AgenticLoopError {
        code: "contextual_grounding_root_required",
        boundary: "VerifiedFilesystemContext",
        message: "OOMU needs one approved Project folder or a signed same-session folder read containing the named input files before it can correct or resolve their paths. Approve that read, then retry. Nothing was changed."
            .to_string(),
        mlc_path: None,
    }
}

fn verified_grounding_context_unavailable() -> AgenticLoopError {
    AgenticLoopError {
        code: "contextual_grounding_context_unavailable",
        boundary: "VerifiedFilesystemContext",
        message: "OOMU could not verify this chat's saved folder approvals. Reopen the Project or approve the folder read again. Nothing was changed."
            .to_string(),
        mlc_path: None,
    }
}

fn rewrite_objective_with_grounded_paths(
    objective: &str,
    references: &[plan_coverage::FileEvidence],
    output_references: &[plan_coverage::FileEvidence],
    grounded: &GroundedObjectivePaths,
) -> Result<String, AgenticLoopError> {
    if references.len() != grounded.inputs.len() {
        return Err(AgenticLoopError {
            code: "contextual_grounding_reference_mismatch",
            boundary: "VerifiedFilesystemContext",
            message: "OOMU couldn’t bind every requested input filename to one verified file. Nothing was changed."
                .to_string(),
            mlc_path: None,
        });
    }
    let mut replacements = references
        .iter()
        .zip(&grounded.inputs)
        .map(|(reference, path)| {
            prompt_safe_path(path).map(|value| (reference.start, reference.end, value))
        })
        .collect::<Result<Vec<_>, _>>()?;
    replacements.extend(
        output_references
            .iter()
            .filter(|reference| {
                let path = Path::new(&reference.path);
                !path.is_absolute()
                    && path.parent().is_some_and(|parent| !parent.as_os_str().is_empty())
            })
            .map(|reference| {
                let name = Path::new(&reference.path)
                    .file_name()
                    .ok_or_else(|| AgenticLoopError {
                        code: "contextual_output_name_invalid",
                        boundary: "VerifiedFilesystemContext",
                        message: "OOMU couldn’t identify one requested output filename safely. Nothing was changed."
                            .to_string(),
                        mlc_path: None,
                    })?;
                let path = grounded.output_directory.join(name);
                prompt_safe_path(&path).map(|value| (reference.start, reference.end, value))
            })
            .collect::<Result<Vec<_>, _>>()?,
    );
    replacements.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
    let mut rewritten = objective.to_string();
    for (start, end, value) in replacements {
        rewritten.replace_range(start..end, &value);
    }
    let output = prompt_safe_path(&grounded.output_directory)?;
    Ok(format!(
        "{rewritten}\n\nLocally verified path grounding: use exact outputDirectory `{output}`. The canonical input paths above supersede relative wording and stale folder spellings."
    ))
}

fn prompt_safe_path(path: &Path) -> Result<String, AgenticLoopError> {
    let value = path.to_string_lossy();
    if value.is_empty()
        || value
            .chars()
            .any(|character| character == '`' || character.is_control())
    {
        return Err(AgenticLoopError {
            code: "contextual_grounded_path_unrepresentable",
            boundary: "VerifiedFilesystemContext",
            message: "A verified path contains characters that cannot be placed safely in an action plan. Nothing was changed."
                .to_string(),
            mlc_path: None,
        });
    }
    Ok(value.into_owned())
}

pub(super) fn contextual_filesystem_route(
    prompt: &str,
    context: &DynamicRoutingContext,
    persistence: Option<&PersistenceEngine>,
    identity: Option<&SovereignIdentity>,
) -> Option<ChatIntentRouteDecision> {
    let session_id = context.session_id.as_deref()?.trim();
    let persistence = persistence?;
    let identity = identity?;
    let starts_new_action = is_contextual_mutation_request(prompt);
    let resumes_filename = !starts_new_action
        && persistence
            .pending_contextual_filename_matches(session_id, prompt, identity)
            .ok()?;
    if !starts_new_action && !resumes_filename {
        return None;
    }
    let target = persistence
        .latest_verified_filesystem_context(session_id, "directory", identity)
        .ok()??;
    let selected_route = [
        context.selected_provider_id.as_deref().unwrap_or("auto"),
        context.selected_model_id.as_deref().unwrap_or("auto"),
    ]
    .join("/");
    let routing_mode = match context.dynamic_routing_override {
        Some(false) => "model-locked",
        _ => "auto-route-compatible",
    };
    Some(ChatIntentRouteDecision {
        route: ChatIntentRoute::AgenticPlanner,
        requires_local_access: true,
        decision_source: "verified_contextual_filesystem_mutation".to_string(),
        reason: format!(
            "The current turn requests a local mutation using one native-verified same-session directory target; Shield approval is still required ({routing_mode}, route {selected_route})."
        ),
        matched_signals: vec![
            if resumes_filename {
                "grounded filename for pending contextual file action".to_string()
            } else {
                "explicit contextual filesystem mutation".to_string()
            },
            format!("verified directory receipt {}", target.verified_receipt_digest),
        ],
        status_label: "OOMU is preparing the exact local change...".to_string(),
    })
}

pub(super) fn is_contextual_mutation_request(prompt: &str) -> bool {
    let normalized = prompt.trim().to_ascii_lowercase();
    if normalized.is_empty()
        || [
            "example",
            "if i said",
            "what does",
            "what would",
            "how would",
            "quoted",
        ]
        .iter()
        .any(|phrase| normalized.contains(phrase))
    {
        return false;
    }
    let has_action = [
        "write",
        "save",
        "create",
        "put",
        "export",
        "record",
        "turn it into",
        "make a file",
    ]
    .iter()
    .any(|phrase| normalized.contains(phrase));
    let has_target = [
        "that folder",
        "this folder",
        "that directory",
        "this directory",
        "same folder",
        "same directory",
        " in there",
        " into there",
        " to there",
    ]
    .iter()
    .any(|phrase| normalized.contains(phrase));
    let directory_only_markdown =
        super::agent_owned_artifact::is_directory_only_markdown_request(prompt);
    (has_action && has_target) || directory_only_markdown
}

pub(super) fn planner_context(
    prompt: &str,
    session_id: Option<&str>,
    persistence: &PersistenceEngine,
    identity: &SovereignIdentity,
) -> Option<String> {
    if !is_contextual_mutation_request(prompt) {
        return None;
    }
    let session_id = session_id?.trim();
    let target = persistence
        .latest_verified_filesystem_context(session_id, "directory", identity)
        .ok()??;
    let content = persistence
        .resolve_assistant_content_reference(session_id, prompt)
        .ok()
        .flatten();
    let mut context = format!(
        "Native verified filesystem reference (not permission):\n- Directory: {}\n- Source turn: {}\n- Receipt digest: {}\n- Required boundary: resolve the final filename, then request Shield approval for the exact canonical path. Never invent a filename or overwrite an existing file.",
        target.canonical_path, target.source_turn_id, target.verified_receipt_digest
    );
    if let Some(content) = content {
        context.push_str(&format!(
            "\n\nUser-selected same-session assistant content:\n- Message ID: {}\n- Content digest: {}\n- Exact content follows:\n{}",
            content.message_id, content.content_digest, content.content
        ));
    }
    Some(context)
}

pub(super) fn deterministic_contextual_file_draft(
    preparation: crate::db::PreparedContextualFileAction,
) -> GeneratedActionPlanDraft {
    let title = Path::new(&preparation.filename)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("document")
        .to_string();
    GeneratedActionPlanDraft {
        steps: vec![GeneratedPlanStepDraft {
            step: format!("Create a new Markdown file at {}.", preparation.destination_path),
            tool: GeneratedToolDraft::RegisteredTaskTool {
                operation: "create_file".to_string(),
                arguments: serde_json::json!({"file": {
                    "title": title,
                    "content": preparation.content,
                    "locale": "en-US",
                    "format": preparation.requested_format,
                    "destinationPath": preparation.destination_path,
                }}),
            },
            risk_level: GeneratedRiskLevel::High,
        }],
        exit_condition: "Exit only after the exact new Markdown file is reopened and its final bytes and content digest are verified.".to_string(),
        generated_text: "Native deterministic contextual file plan".to_string(),
        source: IntentSource::Deterministic,
        degraded_reason: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCENARIO_ONE_SUPPLIER_FILE: &str = "supplier_proposals.json";
    const SCENARIO_ONE_VENDOR_FILE: &str = "q3_strategic_vendor_proposals.txt";
    const SCENARIO_ONE_FIXTURE_DIRECTORY: &str = "mock_data";
    const SCENARIO_ONE_TESTING_DIRECTORY: &str = "testing";
    const SCENARIO_ONE_OUTPUT_DIRECTORY: &str = "ship_test_01";
    const SCENARIO_ONE_RELATIVE_OBJECTIVE: &str = "Prepare a board-ready supplier decision pack. Read `mock_data/supplier_proposals.json` and `mock_data/q3_strategic_vendor_proposals.txt` from my testing folder. Reconcile every quoted amount and margin and identify all exceptions. Independently research current primary or official fuel or freight conditions. Create a new `ship_test_01` folder in the testing folder and deliver supplier_decision.xlsx, supplier_decision.pptx, supplier_decision.pdf, and sources.md. Create a Calendar event and a Mail draft.";

    fn scenario_one_signed_directory(
        suffix: &str,
        receipt_target: &str,
    ) -> (
        PathBuf,
        PathBuf,
        PersistenceEngine,
        SovereignIdentity,
        String,
    ) {
        let base = std::env::temp_dir().join(format!(
            "oomu-scenario-one-paths-{}-{suffix}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        let testing = base.join(SCENARIO_ONE_TESTING_DIRECTORY);
        let fixtures = testing.join(SCENARIO_ONE_FIXTURE_DIRECTORY);
        std::fs::create_dir_all(&fixtures).unwrap();
        std::fs::write(fixtures.join(SCENARIO_ONE_SUPPLIER_FILE), "{}\n").unwrap();
        std::fs::write(fixtures.join(SCENARIO_ONE_VENDOR_FILE), "fixture\n").unwrap();
        let persistence = PersistenceEngine::initialize_at(base.join("state.sqlite")).unwrap();
        let identity = SovereignIdentity::initialize_ephemeral();
        let session = persistence
            .ensure_chat_session(crate::db::CreateChatSessionRequest {
                agent_id: format!("agent-scenario-one-{suffix}"),
                provider_id: "openai".to_string(),
                model_id: "cloud-test".to_string(),
                title: Some("Scenario 1 paths".to_string()),
                dynamic_routing_override: Some(true),
                workspace_id: None,
            })
            .unwrap();
        let turn = ChatTurnPersistenceContext {
            turn_id: format!("turn-scenario-one-{suffix}"),
            generation_token: format!("generation-scenario-one-{suffix}"),
            session_id: session.id.clone(),
            agent_id: session.agent_id,
            provider_id: session.provider_id,
            model_id: session.model_id,
            parent_turn_id: None,
            root_turn_id: format!("turn-scenario-one-{suffix}"),
            turn_kind: "root".to_string(),
        };
        persistence.begin_chat_turn(&turn).unwrap();
        let approved_directory = match receipt_target {
            SCENARIO_ONE_TESTING_DIRECTORY => &testing,
            SCENARIO_ONE_FIXTURE_DIRECTORY => &fixtures,
            _ => panic!("unsupported Scenario 1 receipt target"),
        };
        persistence
            .record_verified_filesystem_context(
                &turn,
                "file_list",
                approved_directory.to_str().unwrap(),
                "directory",
                &identity,
            )
            .unwrap();
        (base, testing, persistence, identity, session.id)
    }

    #[test]
    fn compound_contextual_mutation_is_an_action_but_quoted_examples_are_not() {
        assert!(is_contextual_mutation_request(
            "Take your idea about the contract verification layer and write it into that folder as Markdown."
        ));
        assert!(!is_contextual_mutation_request(
            "What would 'write it into that folder' mean in this example?"
        ));
    }

    #[test]
    fn stale_absolute_input_yields_only_the_nearest_existing_approval_root() {
        let base = std::env::temp_dir().join(format!(
            "oomu-contextual-approval-root-{}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        let testing = base.join("testing");
        let actual = testing.join("mock_data");
        std::fs::create_dir_all(&actual).unwrap();
        std::fs::write(actual.join(SCENARIO_ONE_SUPPLIER_FILE), "{}\n").unwrap();
        std::fs::write(actual.join(SCENARIO_ONE_VENDOR_FILE), "proposal\n").unwrap();
        let stale = testing.join("mocked_data").join(SCENARIO_ONE_SUPPLIER_FILE);
        let objective = format!(
            "Prepare a supplier decision pack. Read {} and {} from my testing folder. Create a new ship_test_01 folder in the testing folder.",
            stale.display(),
            SCENARIO_ONE_VENDOR_FILE
        );
        assert_eq!(
            approval_candidate_root(&objective),
            Some(std::fs::canonicalize(&testing).unwrap())
        );
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn explicit_input_directory_can_ground_relative_named_files() {
        let base = std::env::temp_dir().join(format!(
            "oomu-contextual-input-directory-{}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        let actual = base
            .join("Mobile Documents")
            .join("testing")
            .join("mock_data");
        std::fs::create_dir_all(&actual).unwrap();
        let objective = format!(
            "Prepare a decision pack. Read supplier_proposals.json and q3_strategic_vendor_proposals.txt from {}. Create a new ship_test_01 folder.",
            actual.display()
        );
        assert_eq!(
            plan_coverage::objective_input_directory_references(&objective)
                .into_iter()
                .map(|reference| reference.path)
                .collect::<Vec<_>>(),
            vec![actual.display().to_string()]
        );
        assert_eq!(
            approval_candidate_root(&objective),
            Some(std::fs::canonicalize(&actual).unwrap())
        );
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn published_scenario_paths_resolve_from_signed_testing_or_fixture_directory() {
        for receipt_target in [
            SCENARIO_ONE_TESTING_DIRECTORY,
            SCENARIO_ONE_FIXTURE_DIRECTORY,
        ] {
            let (base, testing, persistence, identity, session_id) =
                scenario_one_signed_directory(receipt_target, receipt_target);
            let resolved = resolve_contextual_objective_paths(
                SCENARIO_ONE_RELATIVE_OBJECTIVE,
                Some(&session_id),
                &persistence,
                &identity,
            )
            .unwrap()
            .expect("the relative Scenario 1 objective should be resolved locally");
            let fixtures = testing.join(SCENARIO_ONE_FIXTURE_DIRECTORY);
            assert!(resolved
                .objective
                .contains(fixtures.join(SCENARIO_ONE_SUPPLIER_FILE).to_str().unwrap()));
            assert!(resolved
                .objective
                .contains(fixtures.join(SCENARIO_ONE_VENDOR_FILE).to_str().unwrap()));
            assert!(resolved.objective.contains(
                testing
                    .join(SCENARIO_ONE_OUTPUT_DIRECTORY)
                    .to_str()
                    .unwrap()
            ));
            assert_eq!(
                resolved.output_directory,
                std::fs::canonicalize(&testing)
                    .unwrap()
                    .join(SCENARIO_ONE_OUTPUT_DIRECTORY)
                    .to_string_lossy()
            );
            assert!(!resolved
                .objective
                .contains("`mock_data/supplier_proposals.json`"));
            std::fs::remove_dir_all(base).unwrap();
        }
    }

    #[test]
    fn project_relative_recovery_input_and_output_ground_beyond_decision_packs() {
        let (base, testing, persistence, identity, session_id) =
            scenario_one_signed_directory("generic-recovery", SCENARIO_ONE_TESTING_DIRECTORY);
        let milestones = testing.join("mock_data").join("project_milestones.json");
        std::fs::write(&milestones, "{}\n").unwrap();
        let objective = "Read `mock_data/project_milestones.json` and construct a recovery plan that respects dependencies. Write the assumptions and critical path to `ship_test_04/recovery_plan.md` and verify the file.";

        let resolved = resolve_contextual_objective_paths(
            objective,
            Some(&session_id),
            &persistence,
            &identity,
        )
        .expect("a Project-scoped non-decision-pack objective should resolve")
        .expect("relative input and output paths require grounding");

        assert!(resolved.objective.contains(milestones.to_str().unwrap()));
        assert!(resolved.objective.contains(
            testing
                .join("ship_test_04")
                .join("recovery_plan.md")
                .to_str()
                .unwrap()
        ));
        assert_eq!(
            resolved.output_directory,
            std::fs::canonicalize(&testing)
                .unwrap()
                .join("ship_test_04")
                .to_string_lossy()
        );
        assert!(!resolved
            .objective
            .contains("`mock_data/project_milestones.json`"));
        assert!(!resolved
            .objective
            .contains("`ship_test_04/recovery_plan.md`"));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn test_eight_folder_named_inside_wording_resolves_to_testing_not_fixture_directory() {
        let (base, testing, persistence, identity, session_id) =
            scenario_one_signed_directory("test-eight-family", SCENARIO_ONE_FIXTURE_DIRECTORY);
        let fixtures = testing.join(SCENARIO_ONE_FIXTURE_DIRECTORY);
        let objective = format!(
            "Prepare a board-ready supplier decision pack using the two files in my testing folder:\n{} and\n{}. Reconcile every quoted amount and margin, identify every exception, and recommend a supplier. Research current fuel or freight conditions that could materially affect the recommendation using primary or official web sources. Create a folder named ship_test_01 inside the testing folder. Deliver supplier_decision.xlsx, supplier_decision.pptx, supplier_decision.pdf, and sources.md. Create a tentative 30-minute event titled Supplier Decision Review in my Family calendar, avoiding conflicts. Create an unsent Mail draft to recipient@example.com listing the four output files.",
            fixtures.join(SCENARIO_ONE_SUPPLIER_FILE).display(),
            fixtures.join(SCENARIO_ONE_VENDOR_FILE).display(),
        );

        let resolved = resolve_contextual_objective_paths(
            &objective,
            Some(&session_id),
            &persistence,
            &identity,
        )
        .expect("Test 8 path grounding should succeed")
        .expect("the relative output folder requires contextual grounding");
        assert_eq!(
            resolved.output_directory,
            std::fs::canonicalize(&testing)
                .unwrap()
                .join(SCENARIO_ONE_OUTPUT_DIRECTORY)
                .to_string_lossy()
        );
        assert!(!resolved
            .output_directory
            .contains("/mock_data/ship_test_01"));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn published_scenario_paths_resolve_from_the_active_project_folder() {
        let (base, testing, persistence, identity, _signed_session_id) =
            scenario_one_signed_directory("project-source", SCENARIO_ONE_FIXTURE_DIRECTORY);
        let project = crate::projects::repository::create(
            &persistence,
            crate::projects::CreateProjectRequest {
                name: "Scenario 1 Project".to_string(),
                description: "Functional Scenario 1 validation".to_string(),
                data_policy: crate::projects::ProjectDataPolicy::AllowConfiguredCloud,
            },
        )
        .unwrap();
        persistence
            .open_connection()
            .unwrap()
            .execute(
                "INSERT INTO project_sources (
                    source_id, project_id, source_kind, canonical_path, grant_reference,
                    created_at_ms, updated_at_ms
                 ) VALUES (?1, ?2, 'local_folder', ?3, ?4, ?5, ?5)",
                rusqlite::params![
                    "source_scenario_one_project",
                    project.project_id,
                    testing.to_string_lossy(),
                    "a".repeat(64),
                    crate::foundation::clock::unix_time_ms_i64(),
                ],
            )
            .unwrap();
        let session = persistence
            .ensure_chat_session(crate::db::CreateChatSessionRequest {
                agent_id: "agent-scenario-one-project".to_string(),
                provider_id: "openai".to_string(),
                model_id: "cloud-test".to_string(),
                title: Some("Scenario 1 project source".to_string()),
                dynamic_routing_override: Some(true),
                workspace_id: None,
            })
            .unwrap();
        crate::projects::repository::bind_record(
            &persistence,
            crate::projects::BindProjectRecordRequest {
                project_id: Some(project.project_id),
                record_kind: "chat_session".to_string(),
                record_id: session.id.clone(),
            },
        )
        .unwrap();

        let resolved = resolve_contextual_objective_paths(
            SCENARIO_ONE_RELATIVE_OBJECTIVE,
            Some(&session.id),
            &persistence,
            &identity,
        )
        .unwrap()
        .expect("active Project access should resolve the published relative paths");
        assert!(resolved.objective.contains(
            testing
                .join(SCENARIO_ONE_FIXTURE_DIRECTORY)
                .join(SCENARIO_ONE_SUPPLIER_FILE)
                .to_str()
                .unwrap()
        ));
        assert!(resolved.objective.contains(
            testing
                .join(SCENARIO_ONE_OUTPUT_DIRECTORY)
                .to_str()
                .unwrap()
        ));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn output_only_relative_path_resolves_inside_the_active_project_folder() {
        let (base, testing, persistence, identity, _signed_session_id) =
            scenario_one_signed_directory("project-output-only", SCENARIO_ONE_FIXTURE_DIRECTORY);
        let project = crate::projects::repository::create(
            &persistence,
            crate::projects::CreateProjectRequest {
                name: "Scenario 4 Project".to_string(),
                description: "Functional Scenario 4 validation".to_string(),
                data_policy: crate::projects::ProjectDataPolicy::AllowConfiguredCloud,
            },
        )
        .unwrap();
        persistence
            .open_connection()
            .unwrap()
            .execute(
                "INSERT INTO project_sources (
                    source_id, project_id, source_kind, canonical_path, grant_reference,
                    created_at_ms, updated_at_ms
                 ) VALUES (?1, ?2, 'local_folder', ?3, ?4, ?5, ?5)",
                rusqlite::params![
                    "source_scenario_four_project",
                    project.project_id,
                    testing.to_string_lossy(),
                    "b".repeat(64),
                    crate::foundation::clock::unix_time_ms_i64(),
                ],
            )
            .unwrap();
        let session = persistence
            .ensure_chat_session(crate::db::CreateChatSessionRequest {
                agent_id: "agent-scenario-four-project".to_string(),
                provider_id: "dynamic".to_string(),
                model_id: "dynamic".to_string(),
                title: Some("Scenario 4 output path".to_string()),
                dynamic_routing_override: Some(true),
                workspace_id: None,
            })
            .unwrap();
        crate::projects::repository::bind_record(
            &persistence,
            crate::projects::BindProjectRecordRequest {
                project_id: Some(project.project_id),
                record_kind: "chat_session".to_string(),
                record_id: session.id.clone(),
            },
        )
        .unwrap();
        let objective = "Research current primary or official sources on scheduled/background agent capabilities in OpenClaw and Claude Cowork. Write a sourced comparison to `ship_test_04/background_agent_comparison.md` in my testing folder. Include URLs, access times, explicit limitations, and a section explaining what this implies for OOMU. Do not claim completion until the file exists and you have read it back.";

        let resolved = resolve_contextual_objective_paths(
            objective,
            Some(&session.id),
            &persistence,
            &identity,
        )
        .unwrap()
        .expect("the active Project must bind an output-only relative path");

        let canonical_testing = std::fs::canonicalize(&testing).unwrap();
        let expected = canonical_testing
            .join("ship_test_04")
            .join("background_agent_comparison.md");
        assert!(resolved.objective.contains(expected.to_str().unwrap()));
        assert_eq!(
            resolved.output_directory,
            canonical_testing.join("ship_test_04").to_string_lossy()
        );
        assert!(!resolved
            .objective
            .contains("`ship_test_04/background_agent_comparison.md`"));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn signed_latest_fixture_correction_supersedes_stale_mocked_data_path() {
        let (base, testing, persistence, identity, session_id) =
            scenario_one_signed_directory("stale-correction", SCENARIO_ONE_FIXTURE_DIRECTORY);
        let stale_supplier = testing.join("mocked_data").join(SCENARIO_ONE_SUPPLIER_FILE);
        let stale = SCENARIO_ONE_RELATIVE_OBJECTIVE.replace(
            "Read `mock_data/supplier_proposals.json` and `mock_data/q3_strategic_vendor_proposals.txt`",
            &format!(
                "Read {} and {}",
                stale_supplier.display(),
                SCENARIO_ONE_VENDOR_FILE
            ),
        );
        let resolved =
            resolve_contextual_objective_paths(&stale, Some(&session_id), &persistence, &identity)
                .unwrap()
                .expect("the signed latest mock_data receipt should supersede the stale spelling");
        assert!(!resolved
            .objective
            .to_ascii_lowercase()
            .contains("mocked_data"));
        assert!(resolved.objective.contains(
            testing
                .join(SCENARIO_ONE_FIXTURE_DIRECTORY)
                .join(SCENARIO_ONE_SUPPLIER_FILE)
                .to_str()
                .unwrap()
        ));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn stale_fixture_path_is_not_corrected_without_a_valid_signed_directory() {
        let (base, _testing, persistence, _signing_identity, session_id) =
            scenario_one_signed_directory("unsigned-correction", SCENARIO_ONE_FIXTURE_DIRECTORY);
        let untrusted_identity = SovereignIdentity::initialize_ephemeral();
        let stale = SCENARIO_ONE_RELATIVE_OBJECTIVE.replace("mock_data", "mocked_data");
        let error = resolve_contextual_objective_paths(
            &stale,
            Some(&session_id),
            &persistence,
            &untrusted_identity,
        )
        .expect_err("a receipt signed by another identity must not authorize correction");
        assert_eq!(error.code, "contextual_grounding_root_required");
        assert!(error.message.contains("signed same-session folder read"));
        assert!(stale.contains("mocked_data"));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test]
    async fn contextual_file_route_uses_a_signed_same_session_directory() {
        let root = std::env::temp_dir().join(format!(
            "oomu-contextual-route-{}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        let folder = root.join("target");
        std::fs::create_dir_all(&folder).unwrap();
        let persistence = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let identity = SovereignIdentity::initialize_ephemeral();
        let session = persistence
            .ensure_chat_session(crate::db::CreateChatSessionRequest {
                agent_id: "agent-context".to_string(),
                provider_id: "local_model".to_string(),
                model_id: "gemma-test".to_string(),
                title: Some("Context route".to_string()),
                dynamic_routing_override: None,
                workspace_id: None,
            })
            .unwrap();
        let turn = ChatTurnPersistenceContext {
            turn_id: "turn-list".to_string(),
            generation_token: "generation-list".to_string(),
            session_id: session.id.clone(),
            agent_id: session.agent_id,
            provider_id: session.provider_id,
            model_id: session.model_id,
            parent_turn_id: None,
            root_turn_id: "turn-list".to_string(),
            turn_kind: "root".to_string(),
        };
        persistence.begin_chat_turn(&turn).unwrap();
        persistence
            .record_verified_filesystem_context(
                &turn,
                "file_list",
                folder.to_str().unwrap(),
                "directory",
                &identity,
            )
            .unwrap();

        let decision = classify_chat_intent_route_for_session(
            ChatIntentRouteRequest {
                prompt: "Take your idea about the contract verification layer and write it into that folder as Markdown.".to_string(),
                automated_web_grounding_enabled: None,
                attachments: vec![],
            },
            DynamicRoutingContext {
                session_id: Some(session.id.clone()),
                dynamic_routing_override: Some(true),
                selected_provider_id: Some("openai".to_string()),
                selected_model_id: Some("gpt-test".to_string()),
            },
            Some(persistence.clone()),
            Some(identity.clone()),
        )
        .await
        .unwrap();
        assert!(matches!(decision.route, ChatIntentRoute::AgenticPlanner));
        assert!(decision.requires_local_access);
        assert_eq!(
            decision.decision_source,
            "verified_contextual_filesystem_mutation"
        );
        persistence
            .insert_chat_message(
                &session.id,
                "agent-context",
                "assistant",
                "Use a contract verification layer with signed boundary receipts.",
            )
            .unwrap();
        assert!(matches!(
            persistence
                .prepare_contextual_file_action(
                    &session.id,
                    "Take your idea about the contract verification layer and write it into that folder. Use markdown format.",
                    true,
                    &identity,
                )
                .unwrap(),
            Some(crate::db::ContextualFileActionPreparation::Ready(_))
        ));
        let filename_decision = classify_chat_intent_route_for_session(
            ChatIntentRouteRequest {
                prompt: "contract-verification-sprint.md".to_string(),
                automated_web_grounding_enabled: None,
                attachments: vec![],
            },
            DynamicRoutingContext {
                session_id: Some(session.id.clone()),
                dynamic_routing_override: Some(true),
                selected_provider_id: Some("openai".to_string()),
                selected_model_id: Some("gpt-test".to_string()),
            },
            Some(persistence.clone()),
            Some(identity.clone()),
        )
        .await
        .unwrap();
        assert!(matches!(
            filename_decision.route,
            ChatIntentRoute::AgenticPlanner
        ));
        assert!(filename_decision
            .matched_signals
            .iter()
            .any(|signal| { signal == "grounded filename for pending contextual file action" }));
        std::fs::remove_dir_all(root).unwrap();
    }
}
