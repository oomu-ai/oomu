use super::AgentManager;
use crate::gemma::{
    resolve_legacy_identity, AutoRouteClassifierHealth, LegacyIdentityResolution,
    StartupModelAssignment,
};
use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::path::Path;
use tauri::Manager;

#[derive(Debug, Clone, Copy)]
pub(crate) enum ModelAssignmentStore {
    Main,
    AttachedOps,
}

#[derive(Debug)]
pub(crate) struct ModelAssignmentAlignment {
    pub updated: usize,
    pub agent_models: HashMap<String, String>,
}

pub(super) fn canonicalize_native_save_request(
    app: &tauri::AppHandle,
    request: &mut super::SaveAgentConfigRequest,
) -> Result<(), String> {
    let provider_id = request.provider_id.as_deref().unwrap_or("local_model");
    if !super::is_local_provider_id(provider_id) {
        return Ok(());
    }
    let model_root = crate::settings::resolved_local_model_directory(app)?;
    request.model_id = if request.model_id.trim().is_empty() {
        let service = app
            .try_state::<crate::gemma::GemmaService>()
            .ok_or_else(implicit_model_choice_required)?;
        implicit_model_id_from_verified_startup(
            service.startup_model_assignment().as_ref(),
            &service.classifier_health(),
        )?
    } else {
        canonical_model_id_for_save(&model_root, &request.model_id)?
    };
    request.provider_id = Some("local_model".to_string());
    Ok(())
}

fn implicit_model_id_from_verified_startup(
    assignment: Option<&StartupModelAssignment>,
    health: &AutoRouteClassifierHealth,
) -> Result<String, String> {
    let assignment = assignment.ok_or_else(implicit_model_choice_required)?;
    if !health.is_ready() || !health.matches_startup_assignment(assignment) {
        return Err(implicit_model_choice_required());
    }
    Ok(assignment.resolved_model_id.clone())
}

fn implicit_model_choice_required() -> String {
    "Choose an installed model for this agent. OOMU could not verify the startup model.".to_string()
}

fn canonical_model_id_for_save(model_root: &Path, model_id: &str) -> Result<String, String> {
    crate::gemma::resolve_canonical_ready_local_model(model_root, model_id)
        .map(|model| model.id)
        .map_err(|error| format!("{}: {}", error.code, error.message))
}

impl AgentManager {
    pub fn align_local_model_assignments(&self, model_root: &Path) -> Result<usize, String> {
        let _guard = self.lock_writes();
        let mut connection = self.open_connection().map_err(|error| error.to_string())?;
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        let alignment = align_model_assignments_in_connection(
            &transaction,
            model_root,
            ModelAssignmentStore::Main,
        )?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(alignment.updated)
    }

    pub(crate) fn resolve_local_model_assignment_for_agent(
        &self,
        agent_id: &str,
        model_root: &Path,
    ) -> Result<LegacyIdentityResolution, String> {
        let Some(config) = self
            .select_agent_config(agent_id)
            .map_err(|error| error.to_string())?
        else {
            return Ok(LegacyIdentityResolution::Unavailable);
        };
        if !super::is_local_provider_id(&config.provider_id) {
            return Ok(LegacyIdentityResolution::Unavailable);
        }
        resolve_legacy_identity(model_root, &config.model_id).map_err(|error| error.message)
    }
}

pub(super) fn ensure_model_identity_schema(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS agent_model_identity_state (
             agent_id TEXT PRIMARY KEY,
             model_id TEXT NOT NULL,
             source TEXT NOT NULL,
             reconciled_at_ms INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS agent_model_assignment_backups (
             agent_id TEXT PRIMARY KEY,
             original_model_id TEXT NOT NULL,
             backed_up_at_ms INTEGER NOT NULL
         );",
    )
}

pub(crate) fn align_model_assignments_in_connection(
    connection: &Connection,
    model_root: &Path,
    store: ModelAssignmentStore,
) -> Result<ModelAssignmentAlignment, String> {
    let assignments = configured_local_model_assignments(connection, store)?;
    let now = crate::foundation::clock::unix_time_ms_i64();
    let mut updated = 0;
    for (agent_id, requested_model_id, existing_source) in assignments {
        let resolution = resolve_legacy_identity(model_root, &requested_model_id)
            .map_err(|error| error.message)?;
        let LegacyIdentityResolution::Unique(identity) = resolution else {
            record_identity_state(
                connection,
                store,
                &agent_id,
                &requested_model_id,
                "needs_user_choice",
                now,
            )?;
            continue;
        };
        let source = if identity.canonical_id != requested_model_id {
            back_up_assignment(connection, store, &agent_id, &requested_model_id, now)?;
            connection
                .execute(
                    &format!(
                        "UPDATE {} SET model_id = ?1, updated_at_ms = ?2 WHERE id = ?3",
                        table(store, "agent_configs")
                    ),
                    params![identity.canonical_id, now, agent_id],
                )
                .map_err(|error| error.to_string())?;
            updated += 1;
            "verified_legacy_repair"
        } else if existing_source == "legacy_unverified" {
            "verified_existing"
        } else {
            existing_source.as_str()
        };
        record_identity_state(
            connection,
            store,
            &agent_id,
            &identity.canonical_id,
            source,
            now,
        )?;
    }
    let agent_models = configured_local_model_assignments(connection, store)?
        .into_iter()
        .map(|(agent_id, model_id, _)| (agent_id, model_id))
        .collect();
    Ok(ModelAssignmentAlignment {
        updated,
        agent_models,
    })
}

fn configured_local_model_assignments(
    connection: &Connection,
    store: ModelAssignmentStore,
) -> Result<Vec<(String, String, String)>, String> {
    let configs = table(store, "agent_configs");
    let identity = table(store, "agent_model_identity_state");
    let mut statement = connection
        .prepare(&format!(
            "SELECT configs.id, configs.model_id,
                    COALESCE(identity.source, 'legacy_unverified')
             FROM {configs} configs
             LEFT JOIN {identity} identity ON identity.agent_id = configs.id
             WHERE lower(replace(configs.provider_id, '-', '_'))
                   IN ('local', 'local_model', 'local_gemma')"
        ))
        .map_err(|error| error.to_string())?;
    let assignments = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    Ok(assignments)
}

fn back_up_assignment(
    connection: &Connection,
    store: ModelAssignmentStore,
    agent_id: &str,
    model_id: &str,
    now: i64,
) -> Result<(), String> {
    connection
        .execute(
            &format!(
                "INSERT OR IGNORE INTO {}
                 (agent_id, original_model_id, backed_up_at_ms) VALUES (?1, ?2, ?3)",
                table(store, "agent_model_assignment_backups")
            ),
            params![agent_id, model_id, now],
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn record_identity_state(
    connection: &Connection,
    store: ModelAssignmentStore,
    agent_id: &str,
    model_id: &str,
    source: &str,
    now: i64,
) -> Result<(), String> {
    connection
        .execute(
            &format!(
                "INSERT INTO {}
                 (agent_id, model_id, source, reconciled_at_ms) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(agent_id) DO UPDATE SET model_id = excluded.model_id,
                     source = excluded.source, reconciled_at_ms = excluded.reconciled_at_ms",
                table(store, "agent_model_identity_state")
            ),
            params![agent_id, model_id, source, now],
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn table(store: ModelAssignmentStore, name: &'static str) -> String {
    match store {
        ModelAssignmentStore::Main => name.to_string(),
        ModelAssignmentStore::AttachedOps => format!("model_ops.{name}"),
    }
}

pub(super) fn record_saved_identity(
    transaction: &rusqlite::Transaction<'_>,
    agent_id: &str,
    model_id: &str,
    source: &str,
    now: i64,
) -> rusqlite::Result<()> {
    transaction.execute(
        "INSERT INTO agent_model_identity_state
         (agent_id, model_id, source, reconciled_at_ms) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(agent_id) DO UPDATE SET model_id = excluded.model_id,
             source = excluded.source, reconciled_at_ms = excluded.reconciled_at_ms",
        params![agent_id, model_id, source, now],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gemma::{
        AutoRouteClassifierStatus, LocalModelIdentity, LocalModelIdentitySource,
        StartupModelSelectionSource, GEMMA_E2B_CANONICAL_ID, GEMMA_E4B_CANONICAL_ID,
    };
    use std::path::PathBuf;

    #[cfg(unix)]
    #[test]
    fn native_agent_save_accepts_a_verified_root_level_e2b_store() {
        let assets = PathBuf::from(crate::OOMU_MANIFEST_DIR).join("../assets/models");
        let installed = assets.join(GEMMA_E2B_CANONICAL_ID);
        if !installed.is_dir() {
            return;
        }
        let parent = std::env::temp_dir().join(format!(
            "oomu-agent-save-root-model-{}-{}",
            std::process::id(),
            crate::foundation::clock::unix_time_ms_i64()
        ));
        let model_root = parent.join("models");
        std::fs::create_dir_all(&parent).expect("create root-level save fixture parent");
        std::os::unix::fs::symlink(&installed, &model_root)
            .expect("link real E2B as root-level save store");

        let model_id = canonical_model_id_for_save(&model_root, GEMMA_E2B_CANONICAL_ID)
            .expect("save the canonical root-level E2B assignment");

        assert_eq!(model_id, GEMMA_E2B_CANONICAL_ID);
        let _ = std::fs::remove_file(&model_root);
        let _ = std::fs::remove_dir_all(&parent);
    }

    fn assignment(model_id: &str) -> StartupModelAssignment {
        StartupModelAssignment {
            requested_model_id: model_id.to_string(),
            resolved_model_id: model_id.to_string(),
            resolved_directory: PathBuf::from("/private/tmp/verified-startup-model"),
            selection_source: StartupModelSelectionSource::ExplicitUserSelection,
            identity: LocalModelIdentity {
                canonical_id: model_id.to_string(),
                display_name: "Verified startup model".to_string(),
                storage_directory: PathBuf::from("/private/tmp/verified-startup-model"),
                source: LocalModelIdentitySource::CanonicalRegistry,
            },
        }
    }

    fn ready_health(assignment: &StartupModelAssignment) -> AutoRouteClassifierHealth {
        AutoRouteClassifierHealth {
            status: AutoRouteClassifierStatus::Ready,
            requested_model_id: Some(assignment.requested_model_id.clone()),
            classifier_model_id: Some(assignment.resolved_model_id.clone()),
            selection_source: Some(assignment.selection_source),
            classifier_version: crate::inference::dynamic_routing::SEMANTIC_CLASSIFIER_VERSION
                .to_string(),
            readiness_generation: 1,
            residency_generation: 1,
            verified_residency_generation: 1,
            last_verified_at_ms: Some(1),
            last_error_code: None,
            last_error_boundary: None,
            redacted_recovery_hint: None,
        }
    }

    #[test]
    fn implicit_agent_choice_uses_the_verified_explicit_e4b_startup_assignment() {
        let assignment = assignment(GEMMA_E4B_CANONICAL_ID);

        assert_eq!(
            implicit_model_id_from_verified_startup(Some(&assignment), &ready_health(&assignment)),
            Ok(GEMMA_E4B_CANONICAL_ID.to_string())
        );
    }

    #[test]
    fn missing_or_ambiguous_startup_identity_fails_closed() {
        let e2b = assignment(GEMMA_E2B_CANONICAL_ID);
        let e4b = assignment(GEMMA_E4B_CANONICAL_ID);
        assert!(implicit_model_id_from_verified_startup(None, &ready_health(&e2b)).is_err());
        assert!(implicit_model_id_from_verified_startup(Some(&e2b), &ready_health(&e4b)).is_err());
    }
}
