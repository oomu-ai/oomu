use super::DecisionPackToolRequest;
use crate::{
    artifacts::decision_pack::{ResearchGap, ResearchGapReason, WebClaim},
    db::PersistenceEngine,
    decision_research_policy::{
        policy_digest, validate_research_policy, ResearchQueryAlternative, ResearchRequirement,
        ResearchSubject,
    },
    sovereign_search::{SovereignSearchAuthorization, SovereignSearchExecutionRequest},
};
use chrono::{SecondsFormat, Utc};
use serde::Deserialize;
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

mod date_evidence;
mod evidence;

const MAX_RESULTS_PER_QUERY: usize = 5;
const TOTAL_RESEARCH_DEADLINE: Duration = Duration::from_secs(65);
const PER_QUERY_DEADLINE: Duration = Duration::from_secs(12);

#[derive(Debug, Deserialize)]
struct SearchContext {
    #[serde(default)]
    results: Vec<SearchResultContext>,
    #[serde(default)]
    pages: Vec<crate::dom_streaming::DomContext>,
}

#[derive(Debug, Deserialize)]
struct SearchResultContext {
    title: String,
    url: String,
    snippet: String,
}

pub(super) struct ResearchOutcome {
    pub(super) claims: Vec<WebClaim>,
    pub(super) gaps: Vec<ResearchGap>,
}

struct RuntimeResearchPolicy<'a> {
    requirement: ResearchRequirement,
    minimum_satisfied_subjects: usize,
    subjects: Vec<RuntimeSubject<'a>>,
    signed_policy_digest: Option<String>,
}

struct RuntimeSubject<'a> {
    subject: ResearchSubject,
    alternatives: Vec<RuntimeAlternative<'a>>,
}

struct RuntimeAlternative<'a> {
    query: &'a str,
    registered: Option<&'a ResearchQueryAlternative>,
}

fn any_of_requirement_is_satisfied(
    requirement: ResearchRequirement,
    minimum_satisfied_subjects: usize,
    claim_count: usize,
) -> bool {
    matches!(requirement, ResearchRequirement::AnyOf) && claim_count >= minimum_satisfied_subjects
}

pub(super) async fn research_official_sources(
    request: &DecisionPackToolRequest,
    plan_id: &str,
    objective: &str,
    verified_input_count: usize,
    session_id: Option<&str>,
    app: &tauri::AppHandle,
    persistence: &PersistenceEngine,
) -> Result<ResearchOutcome, String> {
    let policy = runtime_policy(request, objective)?;
    let started = Instant::now();
    let mut claims = Vec::new();
    let mut gaps = Vec::new();
    let mut seen_urls = HashSet::new();
    let mut total_attempts = 0usize;
    let mut total_pages = 0usize;
    let mut every_attempt_network_unavailable = true;

    for subject in &policy.subjects {
        let mut subject_candidates = Vec::new();
        let mut attempt_count = 0usize;
        let mut page_count = 0usize;
        let mut subject_network_unavailable = true;
        for alternative in &subject.alternatives {
            attempt_count += 1;
            total_attempts += 1;
            let Some(remaining) = TOTAL_RESEARCH_DEADLINE.checked_sub(started.elapsed()) else {
                break;
            };
            let authorization = match (alternative.registered, &request.research_policy) {
                (Some(_), Some(signed_policy)) => {
                    SovereignSearchAuthorization::approved_decision_pack(
                        plan_id,
                        objective,
                        alternative.query,
                        signed_policy.clone(),
                        policy.signed_policy_digest.as_deref().ok_or_else(|| {
                            "Decision-pack signed research policy digest is missing.".to_string()
                        })?,
                    )
                }
                _ => SovereignSearchAuthorization::approved_action_plan(
                    plan_id,
                    objective,
                    alternative.query,
                ),
            };
            let search_request = SovereignSearchExecutionRequest::approved_action_plan(
                alternative.query,
                Some(MAX_RESULTS_PER_QUERY),
                session_id.map(str::to_string),
                authorization,
            );
            let response = match tokio::time::timeout(
                remaining.min(PER_QUERY_DEADLINE),
                crate::sovereign_search::execute_sovereign_duckduckgo_search(
                    search_request,
                    Some(app),
                    Some(persistence.clone()),
                ),
            )
            .await
            {
                Ok(Ok(response)) => response,
                Ok(Err(_)) | Err(_) => continue,
            };
            if response.degraded || response.error_code.is_some() {
                continue;
            }
            subject_network_unavailable = false;
            every_attempt_network_unavailable = false;
            let Ok(context) = serde_json::from_str::<SearchContext>(&response.context_json) else {
                continue;
            };
            // Search snippets are retained only as discovery metadata. They can never
            // become a cited claim without a fetched, qualified page.
            let _discovery_metadata = context
                .results
                .iter()
                .take(MAX_RESULTS_PER_QUERY)
                .filter(|result| {
                    !result.title.trim().is_empty()
                        && !result.url.trim().is_empty()
                        && !result.snippet.trim().is_empty()
                })
                .count();
            page_count += context.pages.len();
            total_pages += context.pages.len();
            let candidates_before = subject_candidates.len();
            let Some(registered) = alternative.registered else {
                subject_candidates.extend(evidence::legacy_claim_candidates(
                    subject.subject,
                    &context.pages,
                    &Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
                ));
                eprintln!(
                    "DECISION_PACK_RESEARCH_ATTEMPT subject={} engine={} pages={} labeled_tables={} claims_added={}",
                    subject.subject.as_str(),
                    response.engine,
                    context.pages.len(),
                    context
                        .pages
                        .iter()
                        .flat_map(|page| page.tables.iter())
                        .filter(|table| !table.label.is_empty())
                        .count(),
                    subject_candidates.len().saturating_sub(candidates_before),
                );
                if subject_candidates.len() > candidates_before {
                    break;
                }
                continue;
            };
            subject_candidates.extend(evidence::claim_candidates(
                subject.subject,
                registered,
                &context.pages,
                &Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
            ));
            eprintln!(
                "DECISION_PACK_RESEARCH_ATTEMPT subject={} authority_profile={} engine={} pages={} labeled_tables={} claims_added={}",
                subject.subject.as_str(),
                registered.authority_profile,
                response.engine,
                context.pages.len(),
                context
                    .pages
                    .iter()
                    .flat_map(|page| page.tables.iter())
                    .filter(|table| !table.label.is_empty())
                    .count(),
                subject_candidates.len().saturating_sub(candidates_before),
            );
            if subject_candidates.len() > candidates_before {
                break;
            }
        }
        subject_candidates.sort_by(|left, right| right.score.cmp(&left.score));
        let selected = subject_candidates.into_iter().find(|candidate| {
            let canonical = evidence::canonical_source_url(&candidate.claim.url);
            !canonical.is_empty() && seen_urls.insert(canonical)
        });
        if let Some(selected) = selected {
            claims.push(selected.claim);
            if any_of_requirement_is_satisfied(
                policy.requirement,
                policy.minimum_satisfied_subjects,
                claims.len(),
            ) {
                break;
            }
        } else {
            gaps.push(ResearchGap {
                subject: subject.subject.as_str().to_string(),
                reason: if subject_network_unavailable {
                    ResearchGapReason::NetworkUnavailable
                } else {
                    ResearchGapReason::EvidenceUnavailable
                },
                attempt_count,
                page_count,
            });
        }
    }

    if claims.len() < policy.minimum_satisfied_subjects {
        let subject = gaps
            .first()
            .map(|gap| gap.subject.as_str())
            .unwrap_or("official research");
        let (code, message) = if every_attempt_network_unavailable {
            (
                "decision_pack_research_network_unavailable",
                "OOMU couldn’t reach the approved official research sources. Your verified inputs are unchanged, and no output files were created.",
            )
        } else {
            (
                "decision_pack_research_evidence_unavailable",
                "OOMU couldn’t verify sufficiently recent official evidence for the required research subject. Your verified inputs are unchanged, and no output files were created.",
            )
        };
        return Err(task_tool_error(
            code,
            message,
            subject,
            total_attempts,
            total_pages,
            verified_input_count,
        ));
    }
    if matches!(policy.requirement, ResearchRequirement::AllOf) && !gaps.is_empty() {
        return Err(task_tool_error(
            "decision_pack_research_evidence_unavailable",
            "OOMU couldn’t verify sufficiently recent official evidence for every required research subject. Your verified inputs are unchanged, and no output files were created.",
            &gaps[0].subject,
            total_attempts,
            total_pages,
            verified_input_count,
        ));
    }
    Ok(ResearchOutcome { claims, gaps })
}

fn runtime_policy<'a>(
    request: &'a DecisionPackToolRequest,
    objective: &str,
) -> Result<RuntimeResearchPolicy<'a>, String> {
    if let Some(policy) = &request.research_policy {
        validate_research_policy(policy)?;
        crate::decision_research_policy::policy_matches_objective(objective, policy)?;
        return Ok(RuntimeResearchPolicy {
            requirement: policy.requirement,
            minimum_satisfied_subjects: policy.minimum_satisfied_subjects,
            subjects: policy
                .subjects
                .iter()
                .map(|subject| RuntimeSubject {
                    subject: subject.subject,
                    alternatives: subject
                        .query_alternatives
                        .iter()
                        .map(|alternative| RuntimeAlternative {
                            query: &alternative.query,
                            registered: Some(alternative),
                        })
                        .collect(),
                })
                .collect(),
            signed_policy_digest: Some(policy_digest(policy)?),
        });
    }

    let objective_policy = crate::decision_research_policy::compile_research_policy(objective)?;
    let mut subjects = Vec::new();
    for query in &request.research_queries {
        let subject = legacy_query_subject(query).ok_or_else(|| {
            "Legacy decision-pack research query has no objective-bound subject.".to_string()
        })?;
        if !objective_policy
            .subjects
            .iter()
            .any(|approved| approved.subject == subject)
        {
            return Err(
                "Legacy decision-pack research query exceeds the original objective.".to_string(),
            );
        }
        if let Some(existing) = subjects
            .iter_mut()
            .find(|existing: &&mut RuntimeSubject<'_>| existing.subject == subject)
        {
            existing.alternatives.push(RuntimeAlternative {
                query,
                registered: None,
            });
        } else {
            subjects.push(RuntimeSubject {
                subject,
                alternatives: vec![RuntimeAlternative {
                    query,
                    registered: None,
                }],
            });
        }
    }
    let requirement = objective_policy.requirement;
    let minimum_satisfied_subjects = match requirement {
        ResearchRequirement::AnyOf => 1,
        ResearchRequirement::AllOf => subjects.len(),
    };
    Ok(RuntimeResearchPolicy {
        requirement,
        minimum_satisfied_subjects,
        subjects,
        signed_policy_digest: None,
    })
}

fn legacy_query_subject(query: &str) -> Option<ResearchSubject> {
    let lowered = query.to_ascii_lowercase();
    if lowered
        .split(|character: char| !character.is_alphanumeric())
        .any(|token| token == "fuel")
    {
        Some(ResearchSubject::Fuel)
    } else if lowered
        .split(|character: char| !character.is_alphanumeric())
        .any(|token| token == "freight")
    {
        Some(ResearchSubject::Freight)
    } else {
        None
    }
}

fn task_tool_error(
    code: &str,
    message: &str,
    subject: &str,
    attempt_count: usize,
    page_count: usize,
    verified_input_count: usize,
) -> String {
    serde_json::json!({
        "taskToolError": {
            "code": code,
            "message": message,
            "context": {
                "subject": subject,
                "attemptCount": attempt_count,
                "pageCount": page_count,
                "verifiedInputCount": verified_input_count,
                "changedState": false
            }
        }
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_any_of_uses_objective_semantics_without_mutating_queries() {
        let request = DecisionPackToolRequest {
            title: "Supplier Decision Pack".to_string(),
            locale: "en-US".to_string(),
            input_paths: vec!["/tmp/input.json".to_string()],
            research_queries: vec![
                "official fuel conditions".to_string(),
                "official freight conditions".to_string(),
            ],
            research_policy: None,
            analysis_instructions: "Reconcile amount, margin, and exceptions.".to_string(),
            output_directory: "/tmp/out".to_string(),
            outputs: super::super::DecisionPackOutputs {
                workbook: "decision.xlsx".to_string(),
                presentation: "decision.pptx".to_string(),
                pdf: "decision.pdf".to_string(),
                sources: "sources.md".to_string(),
            },
            input_bindings: Vec::new(),
            output_binding: None,
        };
        let original = request.research_queries.clone();
        let policy = runtime_policy(
            &request,
            "independently research official web sources for fuel or freight conditions",
        )
        .unwrap();
        assert_eq!(policy.requirement, ResearchRequirement::AnyOf);
        assert_eq!(policy.minimum_satisfied_subjects, 1);
        assert_eq!(request.research_queries, original);
    }

    #[test]
    fn any_of_stops_after_the_first_qualified_subject_but_all_of_does_not() {
        assert!(any_of_requirement_is_satisfied(
            ResearchRequirement::AnyOf,
            1,
            1,
        ));
        assert!(!any_of_requirement_is_satisfied(
            ResearchRequirement::AllOf,
            2,
            1,
        ));
        assert!(!any_of_requirement_is_satisfied(
            ResearchRequirement::AnyOf,
            1,
            0,
        ));
    }

    #[test]
    fn recoverable_error_envelope_is_exact_and_safe() {
        let encoded = task_tool_error(
            "decision_pack_research_evidence_unavailable",
            "Research evidence was unavailable. No files were created.",
            "freight",
            3,
            4,
            2,
        );
        let value: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"taskToolError":{
                "code":"decision_pack_research_evidence_unavailable",
                "message":"Research evidence was unavailable. No files were created.",
                "context":{
                    "subject":"freight",
                    "attemptCount":3,
                    "pageCount":4,
                    "verifiedInputCount":2,
                    "changedState":false
                }
            }})
        );
    }
}
