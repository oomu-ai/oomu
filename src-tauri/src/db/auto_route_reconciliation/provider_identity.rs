use super::*;

pub(in crate::db) fn load_attached_local_provider_configurations(
    transaction: &rusqlite::Transaction<'_>,
) -> rusqlite::Result<Vec<LocalProviderConfiguration>> {
    let mut statement = transaction.prepare(
        "SELECT id, provider_id, custom_model_ids
         FROM model_ops.provider_configs
         WHERE lower(replace(provider_id, '-', '_'))
               IN ('local', 'local_model', 'local_gemma', 'gemma')
         ORDER BY id ASC",
    )?;
    let configurations = statement
        .query_map([], |row| {
            let models = row
                .get::<_, String>(2)?
                .split([',', '\n'])
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect();
            Ok(LocalProviderConfiguration {
                config_id: row.get(0)?,
                provider_type: row.get(1)?,
                model_ids: models,
            })
        })?
        .collect();
    configurations
}

#[derive(Debug)]
struct ProviderIdentityCandidate {
    session_id: String,
    session_provider_id: String,
    session_model_id: String,
    dynamic_routing_override: Option<bool>,
    provider_id: Option<String>,
    model_id: Option<String>,
    reasoning_depth: String,
    context_budget: i32,
    source: String,
    provider_config_id: Option<String>,
    provider_type: Option<String>,
    route_generation: i64,
}

pub(in crate::db) fn reconcile_provider_identities_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    workspace_id: &str,
    model_root: &Path,
    providers: &[LocalProviderConfiguration],
) -> rusqlite::Result<AutoRouteReconciliationReport> {
    let candidates = load_provider_identity_candidates(transaction, workspace_id)?;
    let mut report = AutoRouteReconciliationReport {
        inspected: 0,
        repaired: 0,
        preserved: 0,
        needs_user_choice: 0,
        pending_receipts: Vec::new(),
        pending_migration_integrity: None,
        migration_integrity_verified: false,
    };
    let now = unix_time_ms();
    for candidate in candidates {
        if !provider_identity_candidate_is_local(&candidate, providers) {
            continue;
        }
        report.inspected += 1;
        let Some(model_id) = canonical_candidate_model(model_root, &candidate)? else {
            let unresolved_model = candidate
                .model_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(candidate.session_model_id.trim());
            if mark_provider_identity_choice_required(transaction, &candidate, now)? {
                report
                    .pending_receipts
                    .push(pending_provider_identity_receipt(
                        &candidate,
                        None,
                        None,
                        unresolved_model,
                        candidate.route_generation.max(0),
                        candidate.route_generation.saturating_add(1).max(1),
                        "needs_user_choice",
                        "needs_user_choice",
                    ));
            }
            report.needs_user_choice += 1;
            continue;
        };
        match provider_identity_decision(&candidate, providers, &model_id) {
            ProviderIdentityDecision::Preserve => {
                report
                    .pending_receipts
                    .push(pending_provider_identity_receipt(
                        &candidate,
                        candidate.provider_config_id.clone(),
                        candidate.provider_type.clone(),
                        &model_id,
                        candidate.route_generation,
                        candidate.route_generation,
                        "preserved",
                        candidate.source.as_str(),
                    ));
                report.preserved += 1;
            }
            ProviderIdentityDecision::Repair(provider) => {
                report.pending_receipts.push(repair_provider_identity(
                    transaction,
                    &candidate,
                    provider,
                    &model_id,
                    now,
                )?);
                report.repaired += 1;
            }
            ProviderIdentityDecision::NeedsUserChoice => {
                if mark_provider_identity_choice_required(transaction, &candidate, now)? {
                    report
                        .pending_receipts
                        .push(pending_provider_identity_receipt(
                            &candidate,
                            None,
                            None,
                            &model_id,
                            candidate.route_generation.max(0),
                            candidate.route_generation.saturating_add(1).max(1),
                            "needs_user_choice",
                            "needs_user_choice",
                        ));
                }
                report.needs_user_choice += 1;
            }
        }
    }
    Ok(report)
}

fn load_provider_identity_candidates(
    transaction: &rusqlite::Transaction<'_>,
    workspace_id: &str,
) -> rusqlite::Result<Vec<ProviderIdentityCandidate>> {
    let mut statement = transaction.prepare(
        "SELECT config.session_id, sessions.provider_id, sessions.model_id,
                sessions.dynamic_routing_override, config.provider_id, config.model_id,
                config.reasoning_depth, config.context_budget, config.local_model_source,
                config.local_provider_config_id, config.local_provider_type,
                config.local_route_generation
         FROM active_session_configs config
         JOIN chat_sessions sessions ON sessions.id = config.session_id
         WHERE sessions.workspace_id = ?1",
    )?;
    let candidates = statement
        .query_map(params![workspace_id], |row| {
            Ok(ProviderIdentityCandidate {
                session_id: row.get(0)?,
                session_provider_id: row.get(1)?,
                session_model_id: row.get(2)?,
                dynamic_routing_override: row.get(3)?,
                provider_id: row.get(4)?,
                model_id: row.get(5)?,
                reasoning_depth: row.get(6)?,
                context_budget: row.get(7)?,
                source: row.get(8)?,
                provider_config_id: row.get(9)?,
                provider_type: row.get(10)?,
                route_generation: row.get(11)?,
            })
        })?
        .collect();
    candidates
}

fn canonical_candidate_model(
    model_root: &Path,
    candidate: &ProviderIdentityCandidate,
) -> rusqlite::Result<Option<String>> {
    let model = candidate.model_id.as_deref().unwrap_or_default().trim();
    match resolve_identity(model_root, model)? {
        LegacyIdentityResolution::Unique(identity) => Ok(Some(identity.canonical_id)),
        LegacyIdentityResolution::Ambiguous | LegacyIdentityResolution::Unavailable => Ok(None),
    }
}

fn provider_identity_candidate_is_local(
    candidate: &ProviderIdentityCandidate,
    providers: &[LocalProviderConfiguration],
) -> bool {
    let dynamic = candidate
        .session_provider_id
        .eq_ignore_ascii_case("dynamic")
        && candidate.session_model_id.eq_ignore_ascii_case("dynamic");
    dynamic
        || [
            Some(candidate.session_provider_id.as_str()),
            candidate.provider_id.as_deref(),
            candidate.provider_type.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(is_local_provider)
        || [
            Some(candidate.session_provider_id.as_str()),
            candidate.provider_id.as_deref(),
            candidate.provider_config_id.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|value| providers.iter().any(|provider| provider.config_id == value))
}

enum ProviderIdentityDecision<'a> {
    Preserve,
    Repair(&'a LocalProviderConfiguration),
    NeedsUserChoice,
}

fn provider_identity_decision<'a>(
    candidate: &ProviderIdentityCandidate,
    providers: &'a [LocalProviderConfiguration],
    model_id: &str,
) -> ProviderIdentityDecision<'a> {
    if let Some(config_id) = candidate
        .provider_config_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let Some(provider) = providers
            .iter()
            .find(|provider| provider.config_id == config_id)
        else {
            return ProviderIdentityDecision::NeedsUserChoice;
        };
        if !provider_supports_model(provider, model_id) {
            return ProviderIdentityDecision::NeedsUserChoice;
        }
        if let Some(provider_type) = candidate.provider_type.as_deref() {
            if provider_type != provider.provider_type {
                return ProviderIdentityDecision::NeedsUserChoice;
            }
        } else {
            return ProviderIdentityDecision::Repair(provider);
        }
        let manual_binding_valid = candidate
            .session_provider_id
            .eq_ignore_ascii_case("dynamic")
            || candidate.session_provider_id == provider.config_id;
        if candidate.route_generation > 0
            && manual_binding_valid
            && candidate.provider_id.as_deref() == Some(provider.config_id.as_str())
        {
            return ProviderIdentityDecision::Preserve;
        }
        return ProviderIdentityDecision::Repair(provider);
    }

    let legacy_id = [
        candidate.provider_id.as_deref(),
        Some(candidate.session_provider_id.as_str()),
    ]
    .into_iter()
    .flatten()
    .find_map(|value| {
        providers
            .iter()
            .find(|provider| provider.config_id == value)
    });
    if let Some(provider) = legacy_id {
        return if provider_supports_model(provider, model_id) {
            ProviderIdentityDecision::Repair(provider)
        } else {
            ProviderIdentityDecision::NeedsUserChoice
        };
    }

    let mut matches = providers
        .iter()
        .filter(|provider| provider_supports_model(provider, model_id));
    let Some(provider) = matches.next() else {
        return ProviderIdentityDecision::NeedsUserChoice;
    };
    if matches.next().is_some() {
        ProviderIdentityDecision::NeedsUserChoice
    } else {
        ProviderIdentityDecision::Repair(provider)
    }
}

fn provider_supports_model(provider: &LocalProviderConfiguration, model_id: &str) -> bool {
    provider
        .model_ids
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(model_id))
}

fn repair_provider_identity(
    transaction: &rusqlite::Transaction<'_>,
    candidate: &ProviderIdentityCandidate,
    provider: &LocalProviderConfiguration,
    model_id: &str,
    now: i64,
) -> rusqlite::Result<PendingAutoRouteReconciliationReceipt> {
    back_up_provider_identity_candidate(transaction, candidate, now)?;
    let previous_generation = candidate.route_generation.max(0);
    let current_generation = previous_generation.saturating_add(1).max(1);
    let source = if candidate.source == "legacy_unverified" {
        "verified_legacy_repair"
    } else {
        candidate.source.as_str()
    };
    transaction.execute(
        "UPDATE active_session_configs
         SET provider_id = ?2, model_id = ?3, local_model_source = ?4,
             local_provider_config_id = ?2, local_provider_type = ?5,
             local_route_generation = ?6, local_model_reconciled_at_ms = ?7,
             context_budget = CASE WHEN context_budget > 0 THEN context_budget ELSE ?8 END,
             updated_at = CURRENT_TIMESTAMP
         WHERE session_id = ?1",
        params![
            candidate.session_id,
            provider.config_id,
            model_id,
            source,
            provider.provider_type,
            current_generation,
            now,
            crate::settings::DEFAULT_CONTEXT_BUDGET as i32,
        ],
    )?;
    if !candidate
        .session_provider_id
        .eq_ignore_ascii_case("dynamic")
        && (is_local_provider(&candidate.session_provider_id)
            || candidate.provider_id.as_deref() == Some(candidate.session_provider_id.as_str())
            || providers_match_legacy_binding(provider, &candidate.session_provider_id))
    {
        transaction.execute(
            "UPDATE chat_sessions SET provider_id = ?2, model_id = ?3, updated_at_ms = ?4
             WHERE id = ?1",
            params![candidate.session_id, provider.config_id, model_id, now],
        )?;
    }
    Ok(pending_provider_identity_receipt(
        candidate,
        Some(provider.config_id.clone()),
        Some(provider.provider_type.clone()),
        model_id,
        previous_generation,
        current_generation,
        "repaired",
        source,
    ))
}

fn providers_match_legacy_binding(provider: &LocalProviderConfiguration, value: &str) -> bool {
    provider.config_id == value || provider.provider_type.eq_ignore_ascii_case(value)
}

fn mark_provider_identity_choice_required(
    transaction: &rusqlite::Transaction<'_>,
    candidate: &ProviderIdentityCandidate,
    now: i64,
) -> rusqlite::Result<bool> {
    if candidate.source == "needs_user_choice" {
        return Ok(false);
    }
    back_up_provider_identity_candidate(transaction, candidate, now)?;
    transaction.execute(
        "UPDATE active_session_configs
         SET local_model_source = 'needs_user_choice', local_route_generation = ?2,
             local_model_reconciled_at_ms = ?3, updated_at = CURRENT_TIMESTAMP
         WHERE session_id = ?1",
        params![
            candidate.session_id,
            candidate.route_generation.saturating_add(1).max(1),
            now
        ],
    )?;
    Ok(true)
}

fn back_up_provider_identity_candidate(
    transaction: &rusqlite::Transaction<'_>,
    candidate: &ProviderIdentityCandidate,
    now: i64,
) -> rusqlite::Result<()> {
    transaction.execute(
        "INSERT OR IGNORE INTO auto_route_baseline_backups (
             session_id, provider_id, model_id, reasoning_depth, context_budget,
             local_model_source, backed_up_at_ms, local_provider_config_id,
             local_provider_type, local_route_generation
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            candidate.session_id,
            candidate.provider_id,
            candidate.model_id,
            candidate.reasoning_depth,
            candidate.context_budget,
            candidate.source,
            now,
            candidate.provider_config_id,
            candidate.provider_type,
            candidate.route_generation,
        ],
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn pending_provider_identity_receipt(
    candidate: &ProviderIdentityCandidate,
    provider_config_id: Option<String>,
    provider_type: Option<String>,
    model_id: &str,
    previous_generation: i64,
    current_generation: i64,
    outcome: &'static str,
    provenance: &str,
) -> PendingAutoRouteReconciliationReceipt {
    PendingAutoRouteReconciliationReceipt {
        session_id: candidate.session_id.clone(),
        provider_config_id,
        provider_type,
        model_id: model_id.to_string(),
        provenance: provenance.to_string(),
        previous_generation,
        current_generation,
        outcome,
        dynamic_routing_enabled: candidate
            .session_provider_id
            .eq_ignore_ascii_case("dynamic")
            && candidate.session_model_id.eq_ignore_ascii_case("dynamic")
            && candidate.dynamic_routing_override != Some(false),
    }
}
