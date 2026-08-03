//! Routing and active-session persistence with the parent's connection and write-lock ownership.
use super::{
    session_config_from_row, PersistenceEngine, RoutingPreferenceRecord, SessionConfigRecord,
    UserRoutingPreferenceRecord,
};
use crate::foundation::clock::unix_time_ms_i64 as unix_time_ms;
use rusqlite::{params, OptionalExtension, Row};
use serde::Deserialize;
use serde_json::json;

const MODEL_PRIMARY_ROUTE_KEY: &str = "oomu-primary-route";
const MODEL_FALLBACK_ROUTE_KEY: &str = "oomu-fallback-route";
pub(super) const AUTO_ROUTE_LEGACY_SESSION_CONFIG_FORBIDDEN: &str =
    "auto_route_legacy_session_config_forbidden";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedModelRouteValue {
    #[serde(default)]
    provider_config_id: Option<String>,
    #[serde(default)]
    provider_id: Option<String>,
    #[serde(default)]
    model_id: Option<String>,
    #[serde(default)]
    label: Option<String>,
}

impl PersistenceEngine {
    pub fn select_routing_preference(
        &self,
        key: &str,
    ) -> rusqlite::Result<Option<RoutingPreferenceRecord>> {
        let connection = self.open_connection()?;
        for lookup_key in routing_preference_lookup_keys(key) {
            let record = connection
                .query_row(
                    "SELECT key, value, updated_at FROM routing_preferences WHERE key = ?1",
                    params![lookup_key],
                    routing_preference_from_row,
                )
                .optional()?;
            if record.is_some() {
                return Ok(record);
            }
        }
        Ok(None)
    }

    pub fn select_user_routing_preference(
        &self,
        key: &str,
    ) -> rusqlite::Result<Option<UserRoutingPreferenceRecord>> {
        let connection = self.open_connection()?;
        connection
            .query_row(
                "
                SELECT key, primary_route_id, fallback_route_id, updated_at
                FROM user_routing_preferences
                WHERE key = ?1
                ",
                params![key.trim()],
                user_routing_preference_from_row,
            )
            .optional()
    }

    pub fn upsert_user_routing_preference_pair(
        &self,
        key: &str,
        primary_route_id: &str,
        fallback_route_id: &str,
    ) -> rusqlite::Result<()> {
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        connection.execute(
            "
            INSERT INTO user_routing_preferences (
                key, primary_route_id, fallback_route_id, updated_at
            )
            VALUES (?1, ?2, ?3, CURRENT_TIMESTAMP)
            ON CONFLICT(key) DO UPDATE SET
                primary_route_id = excluded.primary_route_id,
                fallback_route_id = excluded.fallback_route_id,
                updated_at = CURRENT_TIMESTAMP
            ",
            params![
                key.trim(),
                primary_route_id.trim(),
                fallback_route_id.trim()
            ],
        )?;
        Ok(())
    }

    pub fn upsert_user_routing_preference_slot(
        &self,
        key: &str,
        slot: &str,
        route_id: &str,
    ) -> rusqlite::Result<()> {
        let (primary_route_id, fallback_route_id) = match slot.trim().to_lowercase().as_str() {
            "primary" => (Some(route_id.trim()), None),
            "fallback" => (None, Some(route_id.trim())),
            _ => return Ok(()),
        };
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        connection.execute(
            "
            INSERT INTO user_routing_preferences (
                key, primary_route_id, fallback_route_id, updated_at
            )
            VALUES (?1, ?2, ?3, CURRENT_TIMESTAMP)
            ON CONFLICT(key) DO UPDATE SET
                primary_route_id = COALESCE(excluded.primary_route_id, user_routing_preferences.primary_route_id),
                fallback_route_id = COALESCE(excluded.fallback_route_id, user_routing_preferences.fallback_route_id),
                updated_at = CURRENT_TIMESTAMP
            ",
            params![key.trim(), primary_route_id, fallback_route_id],
        )?;
        Ok(())
    }

    pub fn upsert_routing_preference(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        connection.execute(
            "
            INSERT INTO routing_preferences (key, value, updated_at)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(key) DO UPDATE SET
                value = excluded.value,
                updated_at = excluded.updated_at
            ",
            params![key, value, unix_time_ms()],
        )?;
        Ok(())
    }

    pub fn upsert_model_routing_preference(
        &self,
        route_key: &str,
        provider_id: &str,
        provider_config_id: Option<&str>,
        model_id: &str,
        label: Option<&str>,
    ) -> rusqlite::Result<()> {
        let key = canonical_model_route_key(route_key).unwrap_or(route_key);
        let now = unix_time_ms();
        let provider_id = provider_id.trim();
        let provider_config_id = provider_config_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(provider_id);
        let model_id = model_id.trim();
        let label = label
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("{provider_config_id} / {model_id}"));
        let value = json!({
            "providerConfigId": provider_config_id,
            "providerId": provider_id,
            "modelId": model_id,
            "label": label,
            "updatedAt": now,
        })
        .to_string();
        self.upsert_routing_preference(key, &value)?;
        if let Some(slot) = routing_preference_slot(key) {
            let route_id = format!("{provider_config_id}:{model_id}");
            self.upsert_user_routing_preference_slot("default", &slot, &route_id)?;
        }
        Ok(())
    }

    pub fn select_session_config(
        &self,
        session_id: &str,
    ) -> rusqlite::Result<Option<SessionConfigRecord>> {
        let connection = self.open_connection()?;
        connection
            .query_row(
                "
                SELECT session_id, reasoning_depth, context_budget, model_id, updated_at,
                       local_provider_config_id, local_provider_type, local_route_generation
                FROM active_session_configs
                WHERE session_id = ?1
                ",
                params![session_id.trim()],
                session_config_from_row,
            )
            .optional()
    }

    pub fn upsert_session_config(
        &self,
        session_id: &str,
        reasoning_depth: &str,
        context_budget: i32,
        provider_config_id: Option<&str>,
        provider_type: Option<&str>,
        model_id: Option<&str>,
    ) -> rusqlite::Result<()> {
        let provider_config_id = clean_optional_route_text(provider_config_id);
        let provider_type = clean_optional_route_text(provider_type);
        let model_id = clean_optional_route_text(model_id);
        let identity_supplied =
            provider_config_id.is_some() || provider_type.is_some() || model_id.is_some();
        if identity_supplied
            && (provider_config_id.is_none() || provider_type.is_none() || model_id.is_none())
        {
            return Err(rusqlite::Error::InvalidParameterName(
                "session_provider_identity_incomplete".to_string(),
            ));
        }
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let dynamic_bound = transaction.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM chat_sessions
                 WHERE id=?1 AND workspace_id=?2
                   AND lower(provider_id)='dynamic' AND lower(model_id)='dynamic'
                   AND COALESCE(dynamic_routing_override,0)=1
             )",
            params![session_id.trim(), self.workspace_id],
            |row| row.get::<_, bool>(0),
        )?;
        if dynamic_bound && identity_supplied {
            return Err(rusqlite::Error::InvalidParameterName(
                AUTO_ROUTE_LEGACY_SESSION_CONFIG_FORBIDDEN.to_string(),
            ));
        }
        transaction.execute(
            "
            INSERT INTO active_session_configs (
                session_id, reasoning_depth, context_budget, model_id, provider_id, updated_at,
                local_model_source, local_model_reconciled_at_ms,
                local_provider_config_id, local_provider_type, local_route_generation
            )
            VALUES (?1, ?2, ?3, ?4, ?5, CURRENT_TIMESTAMP,
                    CASE WHEN ?4 IS NULL THEN 'legacy_unverified' ELSE 'explicit_session' END,
                    CASE WHEN ?4 IS NULL THEN NULL ELSE ?7 END,
                    ?5, ?6, CASE WHEN ?4 IS NULL THEN 0 ELSE 1 END)
            ON CONFLICT(session_id) DO UPDATE SET
                reasoning_depth = excluded.reasoning_depth,
                context_budget = excluded.context_budget,
                model_id = COALESCE(excluded.model_id, active_session_configs.model_id),
                provider_id = COALESCE(excluded.provider_id, active_session_configs.provider_id),
                local_provider_config_id = COALESCE(
                    excluded.local_provider_config_id,
                    active_session_configs.local_provider_config_id
                ),
                local_provider_type = COALESCE(
                    excluded.local_provider_type,
                    active_session_configs.local_provider_type
                ),
                local_route_generation = CASE
                    WHEN excluded.model_id IS NULL THEN active_session_configs.local_route_generation
                    WHEN active_session_configs.local_provider_config_id = excluded.local_provider_config_id
                     AND active_session_configs.local_provider_type = excluded.local_provider_type
                     AND active_session_configs.model_id = excluded.model_id
                    THEN MAX(active_session_configs.local_route_generation, 1)
                    ELSE MAX(active_session_configs.local_route_generation, 0) + 1
                END,
                local_model_source = CASE
                    WHEN excluded.model_id IS NULL THEN active_session_configs.local_model_source
                    ELSE 'explicit_session'
                END,
                local_model_reconciled_at_ms = CASE
                    WHEN excluded.model_id IS NULL THEN active_session_configs.local_model_reconciled_at_ms
                    ELSE excluded.local_model_reconciled_at_ms
                END,
                updated_at = CURRENT_TIMESTAMP
            ",
            params![
                session_id.trim(),
                reasoning_depth.trim(),
                context_budget,
                model_id,
                provider_config_id,
                provider_type,
                unix_time_ms(),
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }
}

fn clean_optional_route_text(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn routing_preference_lookup_keys(key: &str) -> Vec<String> {
    let key = key.trim();
    match key.to_lowercase().as_str() {
        "primary" | MODEL_PRIMARY_ROUTE_KEY => {
            vec![MODEL_PRIMARY_ROUTE_KEY.to_string(), "primary".to_string()]
        }
        "fallback" | MODEL_FALLBACK_ROUTE_KEY => {
            vec![MODEL_FALLBACK_ROUTE_KEY.to_string(), "fallback".to_string()]
        }
        _ => vec![key.to_string()],
    }
}

pub(super) fn canonical_model_route_key(route_key: &str) -> Option<&'static str> {
    match route_key.trim().to_lowercase().as_str() {
        "primary" | MODEL_PRIMARY_ROUTE_KEY => Some(MODEL_PRIMARY_ROUTE_KEY),
        "fallback" | MODEL_FALLBACK_ROUTE_KEY => Some(MODEL_FALLBACK_ROUTE_KEY),
        _ => None,
    }
}

pub(super) fn routing_preference_from_user_record(
    record: UserRoutingPreferenceRecord,
) -> RoutingPreferenceRecord {
    let primary_route_id = record.primary_route_id.clone();
    let fallback_route_id = record.fallback_route_id.clone();
    let value = json!({
        "primaryRouteId": primary_route_id,
        "fallbackRouteId": fallback_route_id,
    })
    .to_string();
    RoutingPreferenceRecord {
        key: record.key,
        value,
        updated_at: unix_time_ms(),
        primary_route_id,
        fallback_route_id,
        route_key: None,
        provider_id: None,
        provider_config_id: None,
        model_id: None,
        label: None,
    }
}

fn routing_preference_slot(key: &str) -> Option<String> {
    match key.trim().to_lowercase().as_str() {
        "primary" | MODEL_PRIMARY_ROUTE_KEY => Some("primary".to_string()),
        "fallback" | MODEL_FALLBACK_ROUTE_KEY => Some("fallback".to_string()),
        _ => None,
    }
}

fn routing_preference_from_row(row: &Row<'_>) -> rusqlite::Result<RoutingPreferenceRecord> {
    let key = row.get::<_, String>(0)?;
    let value = row.get::<_, String>(1)?;
    let updated_at = row.get::<_, i64>(2)?;
    let parsed = serde_json::from_str::<PersistedModelRouteValue>(&value).ok();
    let provider_id = parsed
        .as_ref()
        .and_then(|route| route.provider_id.as_ref())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let provider_config_id = parsed
        .as_ref()
        .and_then(|route| route.provider_config_id.as_ref())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            provider_id
                .as_ref()
                .filter(|value| value.starts_with("prov-"))
                .cloned()
        });
    let model_id = parsed
        .as_ref()
        .and_then(|route| route.model_id.as_ref())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let label = parsed
        .as_ref()
        .and_then(|route| route.label.as_ref())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    Ok(RoutingPreferenceRecord {
        route_key: routing_preference_slot(&key),
        key,
        value,
        updated_at,
        primary_route_id: None,
        fallback_route_id: None,
        provider_id,
        provider_config_id,
        model_id,
        label,
    })
}

fn user_routing_preference_from_row(
    row: &Row<'_>,
) -> rusqlite::Result<UserRoutingPreferenceRecord> {
    Ok(UserRoutingPreferenceRecord {
        key: row.get(0)?,
        primary_route_id: row.get(1)?,
        fallback_route_id: row.get(2)?,
        updated_at: row.get(3)?,
    })
}
