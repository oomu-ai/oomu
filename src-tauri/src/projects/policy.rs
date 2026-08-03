use super::ProjectDataPolicy;
use crate::{
    db::PersistenceEngine,
    p0_contracts::{ProjectId, TaskId},
};
use rand_core::{OsRng, RngCore};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectTransmissionRequest {
    pub project_id: String,
    pub task_id: Option<String>,
    pub destination_kind: String,
    pub destination_origin: String,
    pub data_classes: Vec<String>,
    #[serde(default)]
    pub consent: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectTransmissionResult {
    pub allowed: bool,
    pub consent_required: bool,
    pub decision_id: String,
    pub policy: ProjectDataPolicy,
    pub redacted_preview: String,
}

/// The Project privacy decision for one resolved inference route.
///
/// `destination_origin` is deliberately the configured route identifier rather
/// than the provider catalog identifier. The catalog identity classifies local
/// versus off-device execution; the route identity binds consent to the exact
/// destination the user reviewed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectProviderPolicyResult {
    pub allowed: bool,
    pub consent_required: bool,
    pub project_id: Option<String>,
    pub destination_origin: String,
    pub decision_id: Option<String>,
    pub policy: Option<ProjectDataPolicy>,
}

fn random_decision_id() -> String {
    let mut bytes = [0_u8; 16];
    OsRng.fill_bytes(&mut bytes);
    format!(
        "decision_{}",
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

fn bounded_provider_identity(value: &str, label: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 256 || trimmed.chars().any(char::is_control) {
        return Err(format!("{label} is invalid."));
    }
    Ok(trimmed.to_string())
}

fn is_local_provider_catalog(provider_id: &str) -> bool {
    matches!(
        provider_id
            .trim()
            .to_ascii_lowercase()
            .replace('-', "_")
            .as_str(),
        "local" | "local_model" | "local_gemma"
    )
}

/// Evaluates the Project boundary after inference routing has resolved both
/// provider identities. Local execution is classified exclusively from the
/// verified catalog identity. Off-device consent remains scoped to the exact
/// configured route identifier so one configured provider cannot authorize a
/// different destination.
pub fn evaluate_project_provider_for_session(
    persistence: &PersistenceEngine,
    session_id: &str,
    route_provider_id: &str,
    catalog_provider_id: &str,
) -> Result<ProjectProviderPolicyResult, String> {
    let destination_origin = bounded_provider_identity(route_provider_id, "Provider route")?;
    let catalog_provider_id =
        bounded_provider_identity(catalog_provider_id, "Provider catalog identity")?;

    if is_local_provider_catalog(&catalog_provider_id) {
        return Ok(ProjectProviderPolicyResult {
            allowed: true,
            consent_required: false,
            project_id: None,
            destination_origin,
            decision_id: None,
            policy: None,
        });
    }

    let Some(context) = persistence.project_inference_context_for_session(session_id)? else {
        return Ok(ProjectProviderPolicyResult {
            allowed: true,
            consent_required: false,
            project_id: None,
            destination_origin,
            decision_id: None,
            policy: None,
        });
    };
    let connection = persistence
        .open_connection()
        .map_err(|error| error.to_string())?;
    let raw_policy = connection
        .query_row(
            "SELECT data_policy FROM project_policy WHERE project_id=?1",
            params![context.project_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Project policy is unavailable; cloud routing is blocked.".to_string())?;
    let policy = match raw_policy.as_str() {
        "local_only" => ProjectDataPolicy::LocalOnly,
        "ask_before_cloud" => ProjectDataPolicy::AskBeforeCloud,
        "allow_configured_cloud" => ProjectDataPolicy::AllowConfiguredCloud,
        _ => return Err("Project policy is invalid; cloud routing is blocked.".to_string()),
    };
    let (allowed, consent_required, recorded_decision) = match policy {
        ProjectDataPolicy::LocalOnly => (false, false, "blocked"),
        ProjectDataPolicy::AskBeforeCloud => (false, true, "consent_required"),
        ProjectDataPolicy::AllowConfiguredCloud => (true, false, "allowed"),
    };
    let decision_id = random_decision_id();
    connection
        .execute(
            "INSERT INTO project_policy_decisions (decision_id, project_id, task_id, destination_kind, destination_origin, data_classes_json, decision, created_at_ms) VALUES (?1, ?2, NULL, 'provider', ?3, '[\"chat_message\",\"project_context\"]', ?4, ?5)",
            params![
                decision_id,
                context.project_id,
                destination_origin,
                recorded_decision,
                crate::foundation::clock::unix_time_ms_i64()
            ],
        )
        .map_err(|error| error.to_string())?;

    Ok(ProjectProviderPolicyResult {
        allowed,
        consent_required,
        project_id: Some(context.project_id),
        destination_origin,
        decision_id: Some(decision_id),
        policy: Some(policy),
    })
}

pub fn evaluate_project_policy(
    persistence: &PersistenceEngine,
    request: ProjectTransmissionRequest,
) -> Result<ProjectTransmissionResult, String> {
    let project_id = ProjectId::parse(request.project_id)?.to_string();
    let task_id = request
        .task_id
        .map(TaskId::parse)
        .transpose()?
        .map(|id| id.to_string());
    let destination_kind = request.destination_kind.trim().to_ascii_lowercase();
    if !matches!(
        destination_kind.as_str(),
        "provider" | "connector" | "remote_mcp" | "browser"
    ) {
        return Err("Unsupported project transmission destination.".to_string());
    }
    let origin = request.destination_origin.trim();
    if origin.is_empty() || origin.len() > 256 {
        return Err("Transmission origin is required.".to_string());
    }
    let mut data_classes = request
        .data_classes
        .into_iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    data_classes.sort();
    data_classes.dedup();
    if data_classes.is_empty() || data_classes.len() > 16 {
        return Err("At least one bounded data class is required.".to_string());
    }
    let connection = persistence
        .open_connection()
        .map_err(|error| error.to_string())?;
    let raw_policy = connection
        .query_row(
            "SELECT data_policy FROM project_policy WHERE project_id=?1",
            params![project_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Project policy is unavailable; transmission is blocked.".to_string())?;
    let policy = match raw_policy.as_str() {
        "local_only" => ProjectDataPolicy::LocalOnly,
        "ask_before_cloud" => ProjectDataPolicy::AskBeforeCloud,
        "allow_configured_cloud" => ProjectDataPolicy::AllowConfiguredCloud,
        _ => return Err("Project policy is invalid; transmission is blocked.".to_string()),
    };
    let (allowed, consent_required, decision) = match policy {
        ProjectDataPolicy::LocalOnly => (false, false, "blocked"),
        ProjectDataPolicy::AskBeforeCloud if !request.consent => (false, true, "consent_required"),
        ProjectDataPolicy::AskBeforeCloud => (true, false, "consented"),
        ProjectDataPolicy::AllowConfiguredCloud => (true, false, "allowed"),
    };
    let decision_id = random_decision_id();
    let now = crate::foundation::clock::unix_time_ms_i64();
    let classes_json = serde_json::to_string(&data_classes).map_err(|error| error.to_string())?;
    connection.execute(
        "INSERT INTO project_policy_decisions (decision_id, project_id, task_id, destination_kind, destination_origin, data_classes_json, decision, created_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![decision_id, project_id, task_id, destination_kind, origin, classes_json, decision, now],
    ).map_err(|error| error.to_string())?;
    Ok(ProjectTransmissionResult {
        allowed,
        consent_required,
        decision_id,
        policy,
        redacted_preview: format!(
            "{} data class(es) to {} via {}",
            data_classes.len(),
            origin,
            destination_kind
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bound_project_session(
        policy: ProjectDataPolicy,
    ) -> (std::path::PathBuf, PersistenceEngine, String, String) {
        let root =
            std::env::temp_dir().join(format!("oomu-project-policy-{}", random_decision_id()));
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let project = crate::projects::repository::create(
            &engine,
            crate::projects::CreateProjectRequest {
                name: "Provider policy test".to_string(),
                description: String::new(),
                data_policy: policy,
            },
        )
        .unwrap();
        let session = engine
            .ensure_chat_session(crate::db::CreateChatSessionRequest {
                agent_id: "agent-test".to_string(),
                provider_id: "local_model".to_string(),
                model_id: "model-test".to_string(),
                title: Some("Project provider policy".to_string()),
                dynamic_routing_override: None,
                workspace_id: None,
            })
            .unwrap();
        crate::projects::repository::bind_record(
            &engine,
            crate::projects::BindProjectRecordRequest {
                project_id: Some(project.project_id.clone()),
                record_kind: "chat_session".to_string(),
                record_id: session.id.clone(),
            },
        )
        .unwrap();
        (root, engine, project.project_id, session.id)
    }

    #[test]
    fn preview_never_contains_private_content() {
        let preview = format!(
            "{} data class(es) to {} via {}",
            2, "api.example.test", "provider"
        );
        assert!(!preview.contains("secret document contents"));
    }

    #[test]
    fn local_only_blocks_before_provider_transmission() {
        let (root, engine, project_id, session_id) =
            bound_project_session(ProjectDataPolicy::LocalOnly);
        let result = evaluate_project_policy(
            &engine,
            ProjectTransmissionRequest {
                project_id,
                task_id: None,
                destination_kind: "provider".to_string(),
                destination_origin: "cloud-provider".to_string(),
                data_classes: vec!["project_context".to_string()],
                consent: false,
            },
        )
        .unwrap();
        assert!(!result.allowed);
        assert!(!result.consent_required);
        assert!(!result.redacted_preview.contains("private content"));
        let decision = evaluate_project_provider_for_session(
            &engine,
            &session_id,
            "prov-cloud-private",
            "google_gemini",
        )
        .unwrap();
        assert!(!decision.allowed);
        assert!(!decision.consent_required);
        assert_eq!(decision.policy, Some(ProjectDataPolicy::LocalOnly));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn opaque_configured_local_route_bypasses_cloud_policy_by_catalog_identity() {
        let (root, engine, _project_id, session_id) =
            bound_project_session(ProjectDataPolicy::LocalOnly);
        let decision = evaluate_project_provider_for_session(
            &engine,
            &session_id,
            "prov-opaque-local-route",
            "local_model",
        )
        .unwrap();
        assert!(decision.allowed);
        assert!(!decision.consent_required);
        assert_eq!(decision.destination_origin, "prov-opaque-local-route");
        assert_eq!(decision.policy, None);
        let decision_count: i64 = engine
            .open_connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM project_policy_decisions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(decision_count, 0);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn genuine_cloud_ask_first_records_scoped_consent_without_authorizing_the_next_turn() {
        let (root, engine, project_id, session_id) =
            bound_project_session(ProjectDataPolicy::AskBeforeCloud);
        let preflight = evaluate_project_provider_for_session(
            &engine,
            &session_id,
            "prov-reviewed-gemini",
            "google_gemini",
        )
        .unwrap();
        assert!(!preflight.allowed);
        assert!(preflight.consent_required);
        assert_eq!(preflight.project_id.as_deref(), Some(project_id.as_str()));
        assert_eq!(preflight.destination_origin, "prov-reviewed-gemini");

        let consent = evaluate_project_policy(
            &engine,
            ProjectTransmissionRequest {
                project_id,
                task_id: None,
                destination_kind: "provider".to_string(),
                destination_origin: "prov-reviewed-gemini".to_string(),
                data_classes: vec!["chat_message".to_string(), "project_context".to_string()],
                consent: true,
            },
        )
        .unwrap();
        assert!(consent.allowed);

        let next_turn = evaluate_project_provider_for_session(
            &engine,
            &session_id,
            "prov-reviewed-gemini",
            "google_gemini",
        )
        .unwrap();
        assert!(!next_turn.allowed);
        assert!(next_turn.consent_required);

        let different_route = evaluate_project_provider_for_session(
            &engine,
            &session_id,
            "prov-unreviewed-gemini",
            "google_gemini",
        )
        .unwrap();
        assert!(!different_route.allowed);
        assert!(different_route.consent_required);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn configured_cloud_policy_allows_the_exact_resolved_route() {
        let (root, engine, _project_id, session_id) =
            bound_project_session(ProjectDataPolicy::AllowConfiguredCloud);
        let decision = evaluate_project_provider_for_session(
            &engine,
            &session_id,
            "prov-configured-cloud",
            "anthropic",
        )
        .unwrap();
        assert!(decision.allowed);
        assert!(!decision.consent_required);
        assert_eq!(
            decision.policy,
            Some(ProjectDataPolicy::AllowConfiguredCloud)
        );
        assert_eq!(decision.destination_origin, "prov-configured-cloud");
        let _ = std::fs::remove_dir_all(root);
    }
}
