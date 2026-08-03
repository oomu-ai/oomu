use super::*;
use crate::gemma::{resolve_legacy_identity, LegacyIdentityResolution, StartupModelAssignment};
use std::{
    collections::{BTreeSet, HashMap},
    path::Path,
};

mod provider_identity;
pub(super) use provider_identity::{
    load_attached_local_provider_configurations, reconcile_provider_identities_in_transaction,
};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AutoRouteReconciliationReport {
    pub inspected: usize,
    pub repaired: usize,
    pub preserved: usize,
    pub needs_user_choice: usize,
    #[serde(skip)]
    pending_receipts: Vec<PendingAutoRouteReconciliationReceipt>,
    #[serde(skip)]
    pending_migration_integrity: Option<PendingAutoRouteMigrationIntegrityReceipt>,
    #[serde(skip)]
    pub(crate) migration_integrity_verified: bool,
}

impl AutoRouteReconciliationReport {
    pub(super) fn absorb(&mut self, other: Self) {
        self.inspected += other.inspected;
        self.repaired += other.repaired;
        self.preserved += other.preserved;
        self.needs_user_choice += other.needs_user_choice;
        self.pending_receipts.extend(other.pending_receipts);
        if self.pending_migration_integrity.is_none() {
            self.pending_migration_integrity = other.pending_migration_integrity;
        }
        self.migration_integrity_verified |= other.migration_integrity_verified;
    }

    pub(super) fn emit_committed_receipts(&mut self) {
        for receipt in self.pending_receipts.drain(..) {
            receipt.emit_committed();
        }
        if let Some(receipt) = self.pending_migration_integrity.take() {
            receipt.emit_committed();
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AutoRouteMigrationIntegritySnapshot {
    session_count: usize,
    message_count: usize,
    turn_count: usize,
    content_digest: String,
    original_routes: HashMap<String, OriginalRouteFields>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OriginalRouteFields {
    provider_id: Option<String>,
    model_id: Option<String>,
    reasoning_depth: String,
    context_budget: i32,
    provenance: String,
    provider_config_id: Option<String>,
    provider_type: Option<String>,
    route_generation: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingAutoRouteMigrationIntegrityReceipt {
    repaired_session_id_hashes: Vec<String>,
    backup_count: usize,
    before: AutoRouteMigrationIntegritySnapshot,
    after: AutoRouteMigrationIntegritySnapshot,
}

impl PendingAutoRouteMigrationIntegrityReceipt {
    fn emit_committed(self) {
        let receipt_id = format!(
            "auto-route-migration-{}",
            &sha256_hex(self.repaired_session_id_hashes.join("\0").as_bytes())[..24],
        );
        crate::diagnostic_output::write_diagnostic_line(format_args!(
            "OOMU_NATIVE_RECEIPT {}",
            serde_json::json!({
                "schemaVersion": 1,
                "kind": "auto_route_migration_integrity",
                "receiptId": receipt_id,
                "committed": true,
                "rolledBack": false,
                "retryable": false,
                "repairedSessionIdHashes": self.repaired_session_id_hashes,
                "backupRowsVerified": true,
                "backupCount": self.backup_count,
                "beforeSessionCount": self.before.session_count,
                "afterSessionCount": self.after.session_count,
                "beforeMessageCount": self.before.message_count,
                "afterMessageCount": self.after.message_count,
                "beforeTurnCount": self.before.turn_count,
                "afterTurnCount": self.after.turn_count,
                "beforeContentDigest": self.before.content_digest,
                "afterContentDigest": self.after.content_digest,
                "noLossVerified": true,
            })
        ));
    }
}

pub(super) fn capture_migration_integrity_snapshot(
    connection: &Connection,
    workspace_id: &str,
) -> rusqlite::Result<AutoRouteMigrationIntegritySnapshot> {
    let mut content = Vec::new();
    let session_count = append_table_snapshot(
        connection,
        workspace_id,
        "chat_sessions",
        &[
            "provider_id",
            "model_id",
            "dynamic_routing_override",
            "updated_at_ms",
        ],
        &mut content,
    )?;
    let message_count =
        append_table_snapshot(connection, workspace_id, "chat_messages", &[], &mut content)?;
    let turn_count =
        append_table_snapshot(connection, workspace_id, "chat_turns", &[], &mut content)?;
    Ok(AutoRouteMigrationIntegritySnapshot {
        session_count,
        message_count,
        turn_count,
        content_digest: sha256_hex(&content),
        original_routes: load_original_route_fields(connection, workspace_id)?,
    })
}

pub(super) fn requires_provider_identity_migration_evidence(
    connection: &Connection,
    workspace_id: &str,
) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM active_session_configs config
             JOIN chat_sessions sessions ON sessions.id=config.session_id
             WHERE sessions.workspace_id=?1
               AND (
                   (lower(sessions.provider_id)='dynamic' AND lower(sessions.model_id)='dynamic')
                   OR lower(replace(config.provider_id,'-','_'))
                      IN ('local','local_model','local_gemma','gemma')
                   OR EXISTS(
                       SELECT 1 FROM model_ops.provider_configs provider
                       WHERE provider.id IN (config.provider_id,config.local_provider_config_id)
                         AND lower(replace(provider.provider_id,'-','_'))
                             IN ('local','local_model','local_gemma','gemma')
                   )
               )
               AND (
                   config.local_provider_config_id IS NULL
                   OR trim(config.local_provider_config_id)=''
                   OR config.local_provider_type IS NULL
                   OR trim(config.local_provider_type)=''
                   OR config.local_route_generation<=0
                   OR trim(COALESCE(config.provider_id,''))
                      <> trim(COALESCE(config.local_provider_config_id,''))
               )
         )",
        params![workspace_id],
        |row| row.get(0),
    )
}

impl AutoRouteReconciliationReport {
    pub(super) fn verify_migration_integrity(
        &mut self,
        connection: &Connection,
        workspace_id: &str,
        before: AutoRouteMigrationIntegritySnapshot,
    ) -> rusqlite::Result<()> {
        let repaired_session_ids = self
            .pending_receipts
            .iter()
            .filter(|receipt| receipt.outcome == "repaired")
            .map(|receipt| receipt.session_id.clone())
            .collect::<BTreeSet<_>>();
        if repaired_session_ids.is_empty() {
            return Ok(());
        }
        let after = capture_migration_integrity_snapshot(connection, workspace_id)?;
        if before.session_count != after.session_count
            || before.message_count != after.message_count
            || before.turn_count != after.turn_count
            || before.content_digest != after.content_digest
        {
            return Err(migration_integrity_error(
                "Auto-route reconciliation changed durable conversation content.",
            ));
        }
        for session_id in &repaired_session_ids {
            let original = before.original_routes.get(session_id).ok_or_else(|| {
                migration_integrity_error(
                    "A repaired Auto-route session was absent from the pre-migration snapshot.",
                )
            })?;
            let backup = load_backed_up_route_fields(connection, session_id)?.ok_or_else(|| {
                migration_integrity_error(
                    "A repaired Auto-route session has no authoritative backup row.",
                )
            })?;
            if &backup != original {
                return Err(migration_integrity_error(
                    "An Auto-route backup row does not match its original route fields.",
                ));
            }
        }
        self.pending_migration_integrity = Some(PendingAutoRouteMigrationIntegrityReceipt {
            repaired_session_id_hashes: repaired_session_ids
                .iter()
                .map(|session_id| sha256_hex(session_id.as_bytes()))
                .collect(),
            backup_count: repaired_session_ids.len(),
            before,
            after,
        });
        self.migration_integrity_verified = true;
        Ok(())
    }
}

fn append_table_snapshot(
    connection: &Connection,
    workspace_id: &str,
    table: &str,
    excluded_columns: &[&str],
    output: &mut Vec<u8>,
) -> rusqlite::Result<usize> {
    let mut metadata =
        connection.prepare(&format!("PRAGMA table_info({})", quote_identifier(table)))?;
    let mut columns = metadata
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, i64>(5)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    columns.retain(|(name, _)| !excluded_columns.contains(&name.as_str()));
    if columns.is_empty() {
        return Err(migration_integrity_error(
            "A required conversation table has no auditable columns.",
        ));
    }
    let primary_keys = {
        let mut keys = columns
            .iter()
            .filter(|(_, position)| *position > 0)
            .collect::<Vec<_>>();
        keys.sort_by_key(|(_, position)| *position);
        keys.into_iter().map(|(name, _)| name).collect::<Vec<_>>()
    };
    let order = if primary_keys.is_empty() {
        columns.iter().map(|(name, _)| name).collect::<Vec<_>>()
    } else {
        primary_keys
    };
    let selections = columns
        .iter()
        .map(|(name, _)| quote_identifier(name))
        .collect::<Vec<_>>()
        .join(",");
    let ordering = order
        .iter()
        .map(|name| quote_identifier(name))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT {selections} FROM {} WHERE workspace_id=?1 ORDER BY {ordering}",
        quote_identifier(table),
    );
    append_digest_segment(output, table.as_bytes());
    for (name, _) in &columns {
        append_digest_segment(output, name.as_bytes());
    }
    let mut statement = connection.prepare(&sql)?;
    let mut rows = statement.query(params![workspace_id])?;
    let mut count = 0usize;
    while let Some(row) = rows.next()? {
        output.push(0xff);
        for index in 0..columns.len() {
            append_sql_value(output, row.get_ref(index)?);
        }
        count += 1;
    }
    Ok(count)
}

fn append_sql_value(output: &mut Vec<u8>, value: rusqlite::types::ValueRef<'_>) {
    match value {
        rusqlite::types::ValueRef::Null => output.push(0),
        rusqlite::types::ValueRef::Integer(value) => {
            output.push(1);
            output.extend_from_slice(&value.to_le_bytes());
        }
        rusqlite::types::ValueRef::Real(value) => {
            output.push(2);
            output.extend_from_slice(&value.to_bits().to_le_bytes());
        }
        rusqlite::types::ValueRef::Text(value) => {
            output.push(3);
            append_digest_segment(output, value);
        }
        rusqlite::types::ValueRef::Blob(value) => {
            output.push(4);
            append_digest_segment(output, value);
        }
    }
}

fn append_digest_segment(output: &mut Vec<u8>, value: &[u8]) {
    output.extend_from_slice(&(value.len() as u64).to_le_bytes());
    output.extend_from_slice(value);
}

fn load_original_route_fields(
    connection: &Connection,
    workspace_id: &str,
) -> rusqlite::Result<HashMap<String, OriginalRouteFields>> {
    let mut statement = connection.prepare(
        "SELECT config.session_id,config.provider_id,config.model_id,
                config.reasoning_depth,config.context_budget,config.local_model_source,
                config.local_provider_config_id,config.local_provider_type,
                config.local_route_generation
         FROM active_session_configs config
         JOIN chat_sessions sessions ON sessions.id=config.session_id
         WHERE sessions.workspace_id=?1 ORDER BY config.session_id",
    )?;
    let rows = statement
        .query_map(params![workspace_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                OriginalRouteFields {
                    provider_id: row.get(1)?,
                    model_id: row.get(2)?,
                    reasoning_depth: row.get(3)?,
                    context_budget: row.get(4)?,
                    provenance: row.get(5)?,
                    provider_config_id: row.get(6)?,
                    provider_type: row.get(7)?,
                    route_generation: row.get(8)?,
                },
            ))
        })?
        .collect();
    rows
}

fn load_backed_up_route_fields(
    connection: &Connection,
    session_id: &str,
) -> rusqlite::Result<Option<OriginalRouteFields>> {
    connection
        .query_row(
            "SELECT provider_id,model_id,reasoning_depth,context_budget,
                    local_model_source,local_provider_config_id,local_provider_type,
                    local_route_generation
             FROM auto_route_baseline_backups WHERE session_id=?1",
            params![session_id],
            |row| {
                Ok(OriginalRouteFields {
                    provider_id: row.get(0)?,
                    model_id: row.get(1)?,
                    reasoning_depth: row.get(2)?,
                    context_budget: row.get(3)?,
                    provenance: row.get(4)?,
                    provider_config_id: row.get(5)?,
                    provider_type: row.get(6)?,
                    route_generation: row.get(7)?,
                })
            },
        )
        .optional()
}

fn migration_integrity_error(message: &str) -> rusqlite::Error {
    rusqlite::Error::InvalidParameterName(message.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingAutoRouteReconciliationReceipt {
    session_id: String,
    provider_config_id: Option<String>,
    provider_type: Option<String>,
    model_id: String,
    provenance: String,
    previous_generation: i64,
    current_generation: i64,
    outcome: &'static str,
    dynamic_routing_enabled: bool,
}

impl PendingAutoRouteReconciliationReceipt {
    fn emit_committed(self) {
        crate::diagnostic_output::write_diagnostic_line(format_args!(
            "OOMU_NATIVE_RECEIPT {}",
            serde_json::json!({
                "kind": "auto_route_reconciliation",
                "receiptId": format!(
                    "auto-route-reconcile-{}-{}-{}",
                    self.session_id, self.current_generation, self.outcome
                ),
                "sessionId": self.session_id,
                "providerConfigId": self.provider_config_id,
                "providerType": self.provider_type,
                "modelId": self.model_id,
                "provenance": self.provenance,
                "outcome": self.outcome,
                "previousRouteGeneration": self.previous_generation,
                "currentRouteGeneration": self.current_generation,
                "dynamicRoutingEnabled": self.dynamic_routing_enabled,
                "committed": true,
                "rolledBack": false,
                "retryable": false,
            })
        ));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LocalProviderConfiguration {
    pub config_id: String,
    pub provider_type: String,
    pub model_ids: Vec<String>,
}

#[derive(Debug)]
struct Candidate {
    session_id: String,
    agent_id: String,
    provider_id: Option<String>,
    model_id: Option<String>,
    reasoning_depth: String,
    context_budget: i32,
    source: String,
    provider_config_id: Option<String>,
    provider_type: Option<String>,
    route_generation: i64,
}

enum CandidateDecision {
    Preserve,
    Repair { model_id: String, source: String },
    NeedsUserChoice,
}

struct AutoRouteRepairRequest<'a> {
    session_id: &'a str,
    turn_id: &'a str,
    generation_token: &'a str,
    provider_config_id: &'a str,
    provider_type: &'a str,
    model_id: &'a str,
}

impl<'a> AutoRouteRepairRequest<'a> {
    fn parse(
        session_id: &'a str,
        turn_id: &'a str,
        generation_token: &'a str,
        provider_config_id: &'a str,
        provider_type: &'a str,
        model_id: &'a str,
    ) -> rusqlite::Result<Self> {
        let request = Self {
            session_id: session_id.trim(),
            turn_id: turn_id.trim(),
            generation_token: generation_token.trim(),
            provider_config_id: provider_config_id.trim(),
            provider_type: provider_type.trim(),
            model_id: model_id.trim(),
        };
        if [
            request.session_id,
            request.turn_id,
            request.generation_token,
            request.provider_config_id,
            request.model_id,
        ]
        .iter()
        .any(|value| value.is_empty())
            || !is_local_provider(request.provider_type)
        {
            return Err(rusqlite::Error::InvalidParameterName(
                "Auto-route repair requires a session and a verified on-device model.".to_string(),
            ));
        }
        Ok(request)
    }
}

impl PersistenceEngine {
    #[cfg(test)]
    pub(crate) fn reconcile_auto_route_session_baselines(
        &self,
        model_root: &Path,
        startup_assignment: &StartupModelAssignment,
        agent_models: &HashMap<String, String>,
    ) -> rusqlite::Result<AutoRouteReconciliationReport> {
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let report = reconcile_session_baselines_in_transaction(
            &transaction,
            &self.workspace_id,
            model_root,
            startup_assignment,
            agent_models,
        )?;
        transaction.commit()?;
        Ok(report)
    }

    pub fn repair_auto_route_session_baseline(
        &self,
        session_id: &str,
        turn_id: &str,
        generation_token: &str,
        local_provider_config_id: &str,
        local_provider_type: &str,
        canonical_model_id: &str,
        model_root: &Path,
    ) -> rusqlite::Result<ChatSessionRoutePolicyRecord> {
        let request = AutoRouteRepairRequest::parse(
            session_id,
            turn_id,
            generation_token,
            local_provider_config_id,
            local_provider_type,
            canonical_model_id,
        )?;
        let model = super::auto_route_validation::canonical_ready_local_baseline(
            model_root,
            request.provider_type,
            request.model_id,
        )?;
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let candidate = transaction.query_row(
            "SELECT config.session_id, sessions.agent_id, config.provider_id,
                    config.model_id, config.reasoning_depth, config.context_budget,
                    config.local_model_source, config.local_provider_config_id,
                    config.local_provider_type, config.local_route_generation
             FROM active_session_configs config
             JOIN chat_sessions sessions ON sessions.id = config.session_id
             WHERE config.session_id = ?1 AND sessions.workspace_id = ?2
               AND lower(sessions.provider_id) = 'dynamic'
               AND lower(sessions.model_id) = 'dynamic'",
            params![request.session_id, &self.workspace_id],
            |row| {
                Ok(Candidate {
                    session_id: row.get(0)?,
                    agent_id: row.get(1)?,
                    provider_id: row.get(2)?,
                    model_id: row.get(3)?,
                    reasoning_depth: row.get(4)?,
                    context_budget: row.get(5)?,
                    source: row.get(6)?,
                    provider_config_id: row.get(7)?,
                    provider_type: row.get(8)?,
                    route_generation: row.get(9)?,
                })
            },
        )?;
        let now = unix_time_ms();
        let generation = candidate.route_generation.saturating_add(1).max(1);
        let changed = repair_exact_frozen_turn_policy(
            &transaction,
            &self.workspace_id,
            model_root,
            request.session_id,
            request.turn_id,
            request.generation_token,
            request.provider_config_id,
            request.provider_type,
            &model,
            candidate.route_generation,
            generation,
            now,
        )?;
        if !changed {
            transaction.commit()?;
            return self
                .select_chat_session_route_policy(request.session_id)?
                .ok_or(rusqlite::Error::QueryReturnedNoRows);
        }
        back_up_candidate(&transaction, &candidate, now)?;
        transaction.execute(
            "UPDATE active_session_configs
             SET provider_id = ?2, model_id = ?3,
                 local_model_source = 'explicit_session',
                 context_budget = CASE WHEN context_budget > 0 THEN context_budget ELSE ?4 END,
                 local_model_reconciled_at_ms = ?5,
                 local_provider_config_id = ?2, local_provider_type = ?6,
                 local_route_generation = ?7, updated_at = CURRENT_TIMESTAMP
             WHERE session_id = ?1",
            params![
                request.session_id,
                request.provider_config_id,
                model,
                crate::settings::DEFAULT_CONTEXT_BUDGET as i32,
                now,
                request.provider_type,
                generation,
            ],
        )?;
        transaction.commit()?;
        crate::diagnostic_output::write_diagnostic_line(format_args!(
            "OOMU_NATIVE_RECEIPT {}",
            serde_json::json!({
                "kind": "auto_route_repair",
                "receiptId": format!("auto-route-repair-{}-{generation}", request.session_id),
                "sessionId": request.session_id,
                "turnId": request.turn_id,
                "providerConfigId": request.provider_config_id,
                "providerType": request.provider_type,
                "modelId": model,
                "provenance": "explicit_session",
                "previousRouteGeneration": candidate.route_generation,
                "currentRouteGeneration": generation,
                "committed": true,
                "rolledBack": false,
                "retryable": false,
            })
        ));
        self.select_chat_session_route_policy(request.session_id)?
            .ok_or(rusqlite::Error::QueryReturnedNoRows)
    }
}

#[allow(clippy::too_many_arguments)]
fn repair_exact_frozen_turn_policy(
    transaction: &rusqlite::Transaction<'_>,
    workspace_id: &str,
    model_root: &Path,
    session_id: &str,
    turn_id: &str,
    generation_token: &str,
    provider_config_id: &str,
    provider_type: &str,
    model: &str,
    previous_route_generation: i64,
    route_generation: i64,
    repaired_at_ms: i64,
) -> rusqlite::Result<bool> {
    let frozen = transaction
        .query_row(
            "SELECT messages.id, messages.metadata_json
             FROM chat_turns turns
             JOIN chat_messages messages ON messages.workspace_id = turns.workspace_id
               AND messages.session_id = turns.session_id AND messages.agent_id = turns.agent_id
               AND messages.role = 'user'
               AND json_extract(messages.metadata_json, '$.turnId') = turns.turn_id
               AND json_extract(messages.metadata_json, '$.generationToken') = turns.generation_token
             WHERE turns.turn_id = ?1 AND turns.generation_token = ?2
               AND turns.workspace_id = ?3 AND turns.session_id = ?4
               AND turns.parent_turn_id IS NULL AND turns.root_turn_id = turns.turn_id
               AND turns.turn_kind = 'root' AND turns.response_claimed_at_ms IS NULL
               AND (turns.status = 'running' OR
                    (turns.status = 'failed' AND turns.completed_at_ms IS NOT NULL
                     AND json_extract(messages.metadata_json, '$.turnState') = 'interrupted'))
               AND (SELECT COUNT(*) FROM chat_messages duplicate
                    WHERE duplicate.workspace_id = turns.workspace_id
                      AND duplicate.session_id = turns.session_id AND duplicate.role = 'user'
                      AND json_extract(duplicate.metadata_json, '$.turnId') = turns.turn_id) = 1",
            params![turn_id, generation_token, workspace_id, session_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()?
        .ok_or_else(repair_turn_rejected)?;
    let mut metadata = frozen
        .1
        .as_deref()
        .map(serde_json::from_str::<Value>)
        .transpose()
        .map_err(json_to_sql_error)?
        .unwrap_or_else(|| json!({}));
    let policy_value = metadata
        .get("autoRoutePolicy")
        .cloned()
        .ok_or_else(repair_turn_rejected)?;
    let mut policy = serde_json::from_value::<AutoRouteTurnPolicyRecord>(policy_value)
        .map_err(json_to_sql_error)?;
    if metadata.get("autoRoutePolicyRepair").is_some() {
        if policy
            .local_provider_id
            .eq_ignore_ascii_case(provider_config_id)
            && policy
                .local_provider_type
                .eq_ignore_ascii_case(provider_type)
            && policy.local_model_id.eq_ignore_ascii_case(model)
            && policy.route_generation == previous_route_generation
        {
            return Ok(false);
        }
        return Err(repair_turn_rejected());
    }
    if policy.local_source != "explicit_session"
        || super::auto_route_validation::canonical_ready_local_baseline(
            model_root,
            &policy.local_provider_type,
            &policy.local_model_id,
        )
        .is_ok()
    {
        return Err(repair_turn_rejected());
    }
    let previous_model_id = policy.local_model_id.clone();
    policy.local_provider_id = provider_config_id.to_string();
    policy.local_provider_type = provider_type.to_string();
    policy.local_model_id = model.to_string();
    policy.local_source = "explicit_session".to_string();
    policy.route_generation = route_generation;
    let object = metadata.as_object_mut().ok_or_else(repair_turn_rejected)?;
    object.insert(
        "autoRoutePolicy".to_string(),
        serde_json::to_value(policy).map_err(json_to_sql_error)?,
    );
    object.insert(
        "autoRoutePolicyRepair".to_string(),
        json!({
            "reason": "unavailable_explicit_session",
            "previousModelId": previous_model_id,
            "repairedModelId": model,
            "repairedAtMs": repaired_at_ms,
        }),
    );
    if transaction.execute(
        "UPDATE chat_messages SET metadata_json = ?2 WHERE id = ?1",
        params![frozen.0, metadata.to_string()],
    )? != 1
    {
        return Err(repair_turn_rejected());
    }
    Ok(true)
}

fn repair_turn_rejected() -> rusqlite::Error {
    rusqlite::Error::InvalidParameterName(
        "Auto-route repair must match one saved turn whose chosen on-device model is unavailable."
            .to_string(),
    )
}

pub(super) fn reconcile_session_baselines_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    workspace_id: &str,
    model_root: &Path,
    startup_assignment: &StartupModelAssignment,
    agent_models: &HashMap<String, String>,
) -> rusqlite::Result<AutoRouteReconciliationReport> {
    let candidates = {
        let mut statement = transaction.prepare(
            "SELECT config.session_id, sessions.agent_id, config.provider_id,
                    config.model_id, config.reasoning_depth, config.context_budget,
                    config.local_model_source, config.local_provider_config_id,
                    config.local_provider_type, config.local_route_generation
             FROM active_session_configs config
             JOIN chat_sessions sessions ON sessions.id = config.session_id
             WHERE sessions.workspace_id = ?1
               AND lower(sessions.provider_id) = 'dynamic'
               AND lower(sessions.model_id) = 'dynamic'",
        )?;
        let rows = statement
            .query_map(params![workspace_id], |row| {
                Ok(Candidate {
                    session_id: row.get(0)?,
                    agent_id: row.get(1)?,
                    provider_id: row.get(2)?,
                    model_id: row.get(3)?,
                    reasoning_depth: row.get(4)?,
                    context_budget: row.get(5)?,
                    source: row.get(6)?,
                    provider_config_id: row.get(7)?,
                    provider_type: row.get(8)?,
                    route_generation: row.get(9)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let mut report = AutoRouteReconciliationReport {
        inspected: candidates.len(),
        repaired: 0,
        preserved: 0,
        needs_user_choice: 0,
        pending_receipts: Vec::new(),
        pending_migration_integrity: None,
        migration_integrity_verified: false,
    };
    let now = unix_time_ms();
    for candidate in candidates {
        let decision =
            candidate_decision(model_root, startup_assignment, agent_models, &candidate)?;
        apply_decision(transaction, &candidate, decision, now, &mut report)?;
    }
    Ok(report)
}

fn candidate_decision(
    model_root: &Path,
    startup_assignment: &StartupModelAssignment,
    agent_models: &HashMap<String, String>,
    candidate: &Candidate,
) -> rusqlite::Result<CandidateDecision> {
    let model = candidate.model_id.as_deref().unwrap_or("").trim();
    let direct = resolve_identity(model_root, model)?;
    match direct {
        LegacyIdentityResolution::Unique(identity) => match candidate.source.as_str() {
            "explicit_session" | "verified_legacy_repair" => {
                preserve_or_repair(candidate, identity.canonical_id, candidate.source.clone())
            }
            "agent_assignment" => {
                agent_assignment_decision(model_root, startup_assignment, agent_models, candidate)
            }
            "startup_default" => {
                startup_assignment_decision(model_root, startup_assignment, candidate)
            }
            _ => preserve_or_repair(
                candidate,
                identity.canonical_id,
                "verified_legacy_repair".to_string(),
            ),
        },
        LegacyIdentityResolution::Ambiguous => match candidate.source.as_str() {
            "agent_assignment" | "legacy_unverified" => {
                ambiguous_assignment_decision(model_root, agent_models, candidate)
            }
            "startup_default" => {
                startup_assignment_decision(model_root, startup_assignment, candidate)
            }
            _ => Ok(CandidateDecision::NeedsUserChoice),
        },
        _ if candidate.source == "explicit_session" => Ok(CandidateDecision::NeedsUserChoice),
        _ => agent_assignment_decision(model_root, startup_assignment, agent_models, candidate),
    }
}

fn ambiguous_assignment_decision(
    model_root: &Path,
    agent_models: &HashMap<String, String>,
    candidate: &Candidate,
) -> rusqlite::Result<CandidateDecision> {
    let Some(agent_model) = agent_models.get(&candidate.agent_id) else {
        return Ok(CandidateDecision::NeedsUserChoice);
    };
    match resolve_identity(model_root, agent_model)? {
        LegacyIdentityResolution::Unique(identity) => preserve_or_repair(
            candidate,
            identity.canonical_id,
            "agent_assignment".to_string(),
        ),
        LegacyIdentityResolution::Ambiguous | LegacyIdentityResolution::Unavailable => {
            Ok(CandidateDecision::NeedsUserChoice)
        }
    }
}

fn agent_assignment_decision(
    model_root: &Path,
    startup_assignment: &StartupModelAssignment,
    agent_models: &HashMap<String, String>,
    candidate: &Candidate,
) -> rusqlite::Result<CandidateDecision> {
    let Some(agent_model) = agent_models.get(&candidate.agent_id) else {
        return startup_assignment_decision(model_root, startup_assignment, candidate);
    };
    match resolve_identity(model_root, agent_model)? {
        LegacyIdentityResolution::Unique(identity) => preserve_or_repair(
            candidate,
            identity.canonical_id,
            "agent_assignment".to_string(),
        ),
        LegacyIdentityResolution::Ambiguous => Ok(CandidateDecision::NeedsUserChoice),
        LegacyIdentityResolution::Unavailable => {
            startup_assignment_decision(model_root, startup_assignment, candidate)
        }
    }
}

fn startup_assignment_decision(
    model_root: &Path,
    startup_assignment: &StartupModelAssignment,
    candidate: &Candidate,
) -> rusqlite::Result<CandidateDecision> {
    match resolve_identity(model_root, startup_assignment.identity.canonical_id())? {
        LegacyIdentityResolution::Unique(identity)
            if identity.canonical_id == startup_assignment.identity.canonical_id =>
        {
            preserve_or_repair(
                candidate,
                identity.canonical_id,
                "startup_default".to_string(),
            )
        }
        _ => Ok(CandidateDecision::NeedsUserChoice),
    }
}

fn preserve_or_repair(
    candidate: &Candidate,
    model_id: String,
    source: String,
) -> rusqlite::Result<CandidateDecision> {
    if baseline_matches(candidate, &model_id, &source) {
        Ok(CandidateDecision::Preserve)
    } else {
        Ok(CandidateDecision::Repair { model_id, source })
    }
}

fn apply_decision(
    transaction: &rusqlite::Transaction<'_>,
    candidate: &Candidate,
    decision: CandidateDecision,
    now: i64,
    report: &mut AutoRouteReconciliationReport,
) -> rusqlite::Result<()> {
    match decision {
        CandidateDecision::Preserve => report.preserved += 1,
        CandidateDecision::NeedsUserChoice => {
            mark_needs_user_choice(transaction, candidate, now)?;
            report.needs_user_choice += 1;
        }
        CandidateDecision::Repair { model_id, source } => {
            repair_candidate(transaction, candidate, &model_id, &source, now)?;
            report.repaired += 1;
        }
    }
    Ok(())
}

fn repair_candidate(
    transaction: &rusqlite::Transaction<'_>,
    candidate: &Candidate,
    model_id: &str,
    source: &str,
    now: i64,
) -> rusqlite::Result<()> {
    back_up_candidate(transaction, candidate, now)?;
    transaction.execute(
        "UPDATE active_session_configs
         SET model_id = ?2, local_model_source = ?3,
             context_budget = CASE WHEN context_budget > 0 THEN context_budget ELSE ?4 END,
             local_model_reconciled_at_ms = ?5,
             local_route_generation = CASE
                 WHEN local_route_generation > 0 THEN local_route_generation + 1 ELSE 1 END,
             updated_at = CURRENT_TIMESTAMP
         WHERE session_id = ?1",
        params![
            candidate.session_id,
            model_id,
            source,
            crate::settings::DEFAULT_CONTEXT_BUDGET as i32,
            now
        ],
    )?;
    Ok(())
}

fn resolve_identity(model_root: &Path, value: &str) -> rusqlite::Result<LegacyIdentityResolution> {
    resolve_legacy_identity(model_root, value)
        .map_err(|error| rusqlite::Error::InvalidParameterName(error.message))
}

fn baseline_matches(candidate: &Candidate, model_id: &str, source: &str) -> bool {
    candidate.model_id.as_deref() == Some(model_id)
        && candidate.source == source
        && candidate.context_budget > 0
}

fn mark_needs_user_choice(
    transaction: &rusqlite::Transaction<'_>,
    candidate: &Candidate,
    now: i64,
) -> rusqlite::Result<()> {
    if candidate.source == "needs_user_choice" {
        return Ok(());
    }
    back_up_candidate(transaction, candidate, now)?;
    transaction.execute(
        "UPDATE active_session_configs
         SET local_model_source = 'needs_user_choice',
             local_model_reconciled_at_ms = ?2,
             local_route_generation = CASE
                 WHEN local_route_generation > 0 THEN local_route_generation + 1 ELSE 1 END,
             updated_at = CURRENT_TIMESTAMP
         WHERE session_id = ?1",
        params![candidate.session_id, now],
    )?;
    Ok(())
}

fn back_up_candidate(
    transaction: &rusqlite::Transaction<'_>,
    candidate: &Candidate,
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

fn is_local_provider(provider: &str) -> bool {
    matches!(
        provider
            .trim()
            .replace('-', "_")
            .to_ascii_lowercase()
            .as_str(),
        "local" | "local_model" | "local_gemma" | "gemma"
    )
}
