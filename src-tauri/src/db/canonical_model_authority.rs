use super::{auto_route_reconciliation, get_database_key, PersistenceEngine};
use crate::agent_manager::{
    model_assignments::{align_model_assignments_in_connection, ModelAssignmentStore},
    AgentManager,
};
use crate::gemma::{resolve_legacy_identity, LegacyIdentityResolution, StartupModelAssignment};
use rusqlite::{params, Connection, TransactionBehavior};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CanonicalModelAuthorityReport {
    pub aligned_agents: usize,
    pub sessions: auto_route_reconciliation::AutoRouteReconciliationReport,
}

struct SessionAuthorityRow {
    session_provider: String,
    session_model: String,
    legacy_provider: Option<String>,
    model: Option<String>,
    source: String,
    provider_config_id: Option<String>,
    provider_type: Option<String>,
    generation: i64,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy)]
pub(crate) enum AuthorityMigrationFailurePoint {
    AfterAgentAlignment,
    AfterSessionAlignment,
}

impl PersistenceEngine {
    pub(crate) fn reconcile_canonical_model_authorities(
        &self,
        agent_manager: &AgentManager,
        model_root: &Path,
        startup_assignment: &StartupModelAssignment,
    ) -> Result<CanonicalModelAuthorityReport, String> {
        self.reconcile_canonical_model_authorities_inner(
            agent_manager,
            model_root,
            startup_assignment,
            None,
        )
    }

    #[cfg(test)]
    pub(crate) fn reconcile_canonical_model_authorities_with_failure(
        &self,
        agent_manager: &AgentManager,
        model_root: &Path,
        startup_assignment: &StartupModelAssignment,
        failure: AuthorityMigrationFailurePoint,
    ) -> Result<CanonicalModelAuthorityReport, String> {
        self.reconcile_canonical_model_authorities_inner(
            agent_manager,
            model_root,
            startup_assignment,
            Some(failure),
        )
    }

    fn reconcile_canonical_model_authorities_inner(
        &self,
        agent_manager: &AgentManager,
        model_root: &Path,
        startup_assignment: &StartupModelAssignment,
        #[cfg(test)] failure: Option<AuthorityMigrationFailurePoint>,
        #[cfg(not(test))] _failure: Option<()>,
    ) -> Result<CanonicalModelAuthorityReport, String> {
        let _agent_guard = agent_manager.lock_writes();
        let _state_guard = self.lock_writes();
        let ops_path = PathBuf::from(agent_manager.db_path());
        if ops_path != self.ops_db_path() {
            return Err(
                "The model authority stores do not share one application profile.".to_string(),
            );
        }
        let key = get_database_key()?;
        let mut state = self
            .open_connection_with_key(&key)
            .map_err(|error| error.to_string())?;
        let original_state_mode = journal_mode(&state, "main")?;
        let original_ops_mode = prepare_ops_journal(&ops_path, &key)?;
        if let Err(error) = prepare_attached_store(&state, &ops_path, &key) {
            let cleanup = cleanup_unattached_store(
                &state,
                &ops_path,
                &key,
                &original_state_mode,
                &original_ops_mode,
            );
            return Err(match cleanup {
                Ok(()) => error,
                Err(cleanup_error) => {
                    format!("{error} Journal restore also failed: {cleanup_error}")
                }
            });
        }

        let result: Result<CanonicalModelAuthorityReport, String> = (|| {
            let transaction = state
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|error| error.to_string())?;
            let migration_before =
                auto_route_reconciliation::requires_provider_identity_migration_evidence(
                    &transaction,
                    &self.workspace_id,
                )
                .map_err(|error| error.to_string())?
                .then(|| {
                    auto_route_reconciliation::capture_migration_integrity_snapshot(
                        &transaction,
                        &self.workspace_id,
                    )
                })
                .transpose()
                .map_err(|error| error.to_string())?;
            let agents = align_model_assignments_in_connection(
                &transaction,
                model_root,
                ModelAssignmentStore::AttachedOps,
            )?;
            #[cfg(test)]
            if matches!(
                failure,
                Some(AuthorityMigrationFailurePoint::AfterAgentAlignment)
            ) {
                return Err("injected authority migration failure after agents".to_string());
            }
            let mut sessions =
                auto_route_reconciliation::reconcile_session_baselines_in_transaction(
                    &transaction,
                    &self.workspace_id,
                    model_root,
                    startup_assignment,
                    &agents.agent_models,
                )
                .map_err(|error| error.to_string())?;
            let provider_configurations =
                auto_route_reconciliation::load_attached_local_provider_configurations(
                    &transaction,
                )
                .map_err(|error| error.to_string())?;
            let provider_sessions =
                auto_route_reconciliation::reconcile_provider_identities_in_transaction(
                    &transaction,
                    &self.workspace_id,
                    model_root,
                    &provider_configurations,
                )
                .map_err(|error| error.to_string())?;
            sessions.absorb(provider_sessions);
            #[cfg(test)]
            if matches!(
                failure,
                Some(AuthorityMigrationFailurePoint::AfterSessionAlignment)
            ) {
                return Err("injected authority migration failure after sessions".to_string());
            }
            verify_authorities(&transaction, &self.workspace_id, model_root)?;
            if let Some(migration_before) = migration_before {
                sessions
                    .verify_migration_integrity(&transaction, &self.workspace_id, migration_before)
                    .map_err(|error| error.to_string())?;
            }
            transaction.commit().map_err(|error| error.to_string())?;
            sessions.emit_committed_receipts();
            Ok(CanonicalModelAuthorityReport {
                aligned_agents: agents.updated,
                sessions,
            })
        })();

        let cleanup = cleanup_attached_store(
            &state,
            &ops_path,
            &key,
            &original_state_mode,
            &original_ops_mode,
        );
        finalize_authority_reconciliation(result, cleanup)
    }
}

fn finalize_authority_reconciliation(
    result: Result<CanonicalModelAuthorityReport, String>,
    cleanup: Result<(), String>,
) -> Result<CanonicalModelAuthorityReport, String> {
    match result {
        Err(error) => Err(match cleanup {
            Ok(()) => error,
            Err(cleanup_error) => {
                format!("{error} Journal restore also failed: {cleanup_error}")
            }
        }),
        Ok(report) => {
            if let Err(error) = cleanup {
                eprintln!("CANONICAL_MODEL_AUTHORITY_JOURNAL_RESTORE_FAILED {error}");
            }
            Ok(report)
        }
    }
}

fn verify_authorities(
    transaction: &rusqlite::Transaction<'_>,
    workspace_id: &str,
    model_root: &Path,
) -> Result<(), String> {
    let invalid_agents: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM model_ops.agent_configs configs
             LEFT JOIN model_ops.agent_model_identity_state identity
               ON identity.agent_id = configs.id
             WHERE lower(replace(configs.provider_id, '-', '_'))
                   IN ('local', 'local_model', 'local_gemma')
               AND (identity.agent_id IS NULL OR identity.model_id <> configs.model_id)",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if invalid_agents != 0 {
        return Err("Agent model identity verification failed.".to_string());
    }
    let providers =
        auto_route_reconciliation::load_attached_local_provider_configurations(transaction)
            .map_err(|error| error.to_string())?;
    let mut statement = transaction
        .prepare(
            "SELECT sessions.provider_id, sessions.model_id, config.provider_id,
                    config.model_id, config.local_model_source,
                    config.local_provider_config_id, config.local_provider_type,
                    config.local_route_generation
             FROM active_session_configs config
             JOIN chat_sessions sessions ON sessions.id = config.session_id
             WHERE sessions.workspace_id = ?1",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![workspace_id], |row| {
            Ok(SessionAuthorityRow {
                session_provider: row.get(0)?,
                session_model: row.get(1)?,
                legacy_provider: row.get(2)?,
                model: row.get(3)?,
                source: row.get(4)?,
                provider_config_id: row.get(5)?,
                provider_type: row.get(6)?,
                generation: row.get(7)?,
            })
        })
        .map_err(|error| error.to_string())?;
    for row in rows {
        verify_session_authority(
            row.map_err(|error| error.to_string())?,
            &providers,
            model_root,
        )?;
    }
    Ok(())
}

fn verify_session_authority(
    row: SessionAuthorityRow,
    providers: &[auto_route_reconciliation::LocalProviderConfiguration],
    model_root: &Path,
) -> Result<(), String> {
    let dynamic = row.session_provider.eq_ignore_ascii_case("dynamic")
        && row.session_model.eq_ignore_ascii_case("dynamic");
    let local = dynamic
        || [
            Some(row.session_provider.as_str()),
            row.legacy_provider.as_deref(),
            row.provider_type.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(super::auto_route_validation::is_local_provider)
        || [
            Some(row.session_provider.as_str()),
            row.legacy_provider.as_deref(),
            row.provider_config_id.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|value| providers.iter().any(|provider| provider.config_id == value));
    if !local {
        return Ok(());
    }
    if (!dynamic && row.provider_config_id.is_none()) || row.source == "needs_user_choice" {
        return Ok(());
    }
    let model = row.model.unwrap_or_default();
    let resolved = resolve_legacy_identity(model_root, &model).map_err(|error| error.message)?;
    let provider_config_id = row
        .provider_config_id
        .ok_or_else(|| "Saved chat provider configuration verification failed.".to_string())?;
    let provider = providers
        .iter()
        .find(|provider| provider.config_id == provider_config_id)
        .ok_or_else(|| "Saved chat provider configuration verification failed.".to_string())?;
    let model_belongs_to_provider = provider
        .model_ids
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(&model));
    let manual_binding_valid = dynamic || row.session_provider == provider.config_id;
    let canonical_model_valid = matches!(
        resolved,
        LegacyIdentityResolution::Unique(identity) if identity.canonical_id == model
    );
    if row.legacy_provider.as_deref() != Some(provider.config_id.as_str())
        || row.provider_type.as_deref() != Some(provider.provider_type.as_str())
        || row.generation <= 0
        || !manual_binding_valid
        || !model_belongs_to_provider
        || !canonical_model_valid
    {
        return Err("Saved chat model identity verification failed.".to_string());
    }
    Ok(())
}

fn prepare_ops_journal(path: &Path, key: &str) -> Result<String, String> {
    let ops = super::open_ops_database_connection_with_key(path, key)
        .map_err(|error| error.to_string())?;
    ops.execute_batch("PRAGMA wal_checkpoint(FULL);")
        .map_err(|error| error.to_string())?;
    let original = journal_mode(&ops, "main")?;
    set_journal_mode(&ops, "main", "delete")?;
    Ok(original)
}

fn prepare_attached_store(state: &Connection, ops_path: &Path, key: &str) -> Result<(), String> {
    state
        .execute_batch("PRAGMA wal_checkpoint(FULL);")
        .map_err(|error| error.to_string())?;
    set_journal_mode(state, "main", "delete")?;
    state
        .execute(
            "ATTACH DATABASE ?1 AS model_ops KEY ?2",
            params![ops_path.to_string_lossy(), key],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn cleanup_unattached_store(
    state: &Connection,
    ops_path: &Path,
    key: &str,
    state_mode: &str,
    ops_mode: &str,
) -> Result<(), String> {
    let state_restore = set_journal_mode(state, "main", state_mode);
    let ops_restore = restore_ops_journal(ops_path, key, ops_mode);
    state_restore.and(ops_restore)
}

fn cleanup_attached_store(
    state: &Connection,
    ops_path: &Path,
    key: &str,
    state_mode: &str,
    ops_mode: &str,
) -> Result<(), String> {
    state
        .execute_batch("DETACH DATABASE model_ops;")
        .map_err(|error| error.to_string())?;
    set_journal_mode(state, "main", state_mode)?;
    restore_ops_journal(ops_path, key, ops_mode)
}

fn restore_ops_journal(ops_path: &Path, key: &str, mode: &str) -> Result<(), String> {
    let ops = super::open_ops_database_connection_with_key(ops_path, key)
        .map_err(|error| error.to_string())?;
    set_journal_mode(&ops, "main", mode)
}

fn journal_mode(connection: &Connection, schema: &str) -> Result<String, String> {
    connection
        .query_row(&format!("PRAGMA {schema}.journal_mode"), [], |row| {
            row.get(0)
        })
        .map_err(|error| error.to_string())
}

fn set_journal_mode(connection: &Connection, schema: &str, mode: &str) -> Result<(), String> {
    if !matches!(mode, "delete" | "wal" | "truncate" | "persist") {
        return Err("Unsupported database journal mode.".to_string());
    }
    let actual: String = connection
        .query_row(
            &format!("PRAGMA {schema}.journal_mode = {mode}"),
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if actual.eq_ignore_ascii_case(mode) {
        Ok(())
    } else {
        Err("The model authority stores could not enter one atomic transaction.".to_string())
    }
}
