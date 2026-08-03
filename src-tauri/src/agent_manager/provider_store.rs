use super::{is_local_provider_id, ConfiguredProvider, Connection};
use crate::secret_store;
use rusqlite::{params, OptionalExtension};
use std::io;

pub(super) fn column_exists(
    connection: &Connection,
    table: &str,
    column: &str,
) -> rusqlite::Result<bool> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement.query_map([], |row| row.get::<_, String>(1))?;
    for existing in columns {
        if existing? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn clean_provider_api_key_input(value: Option<&str>) -> Option<String> {
    let trimmed = value?.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("masked")
        || trimmed.eq_ignore_ascii_case("[masked]")
        || trimmed
            .chars()
            .all(|character| matches!(character, '*' | '•' | '·' | '●'))
    {
        return None;
    }
    Some(trimmed.to_string())
}

pub(super) fn credential_store_error(code: String) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(io::Error::new(io::ErrorKind::Other, code)))
}

pub(super) fn hydrate_selected_provider_secret(
    connection: &Connection,
    mut config: ConfiguredProvider,
    legacy_secret: Option<String>,
) -> rusqlite::Result<ConfiguredProvider> {
    if is_local_provider_id(&config.provider_id) {
        config.credential_configured = false;
        return Ok(config);
    }

    let mut api_key =
        secret_store::get_provider_secret(&config.id).map_err(credential_store_error)?;
    if api_key.is_none() {
        if let Some(secret) = clean_provider_api_key_input(legacy_secret.as_deref()) {
            secret_store::set_provider_secret(&config.id, &secret)
                .map_err(credential_store_error)?;
            api_key = Some(secret);
        }
    }
    if legacy_secret.is_some() {
        connection.execute(
            "UPDATE provider_configs SET api_key = NULL, credential_configured = ?2 WHERE id = ?1",
            params![config.id.as_str(), i64::from(api_key.is_some())],
        )?;
    }
    config.credential_configured = api_key.is_some();
    config.api_key = api_key;
    Ok(config)
}

pub(super) fn reconcile_provider_credential_metadata(
    connection: &Connection,
    config: &mut ConfiguredProvider,
    legacy_secret_present: bool,
) -> rusqlite::Result<()> {
    if is_local_provider_id(&config.provider_id) {
        config.credential_configured = false;
        return Ok(());
    }
    if !config.credential_configured || legacy_secret_present {
        return Ok(());
    }
    if matches!(secret_store::provider_secret_exists(&config.id), Ok(false)) {
        connection.execute(
            "UPDATE provider_configs SET credential_configured = 0 WHERE id = ?1",
            params![config.id.as_str()],
        )?;
        config.credential_configured = false;
    }
    Ok(())
}

pub(super) fn select_provider_configs(
    connection: &Connection,
) -> rusqlite::Result<Vec<ConfiguredProvider>> {
    let mut statement = connection.prepare(
        "
        SELECT id, provider_id, provider_name, auth_method, base_url, api_key_label,
               CASE WHEN credential_configured = 1 OR (api_key IS NOT NULL AND trim(api_key) != '') THEN 1 ELSE 0 END,
               custom_model_ids, auto_route_target, created_at_ms, updated_at_ms, api_key
        FROM provider_configs
        ORDER BY created_at_ms DESC
        ",
    )?;
    let rows = statement.query_map([], provider_metadata_row)?;
    let mut providers = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    for (config, legacy_secret_present) in &mut providers {
        reconcile_provider_credential_metadata(connection, config, *legacy_secret_present)?;
    }
    Ok(providers.into_iter().map(|(config, _)| config).collect())
}

pub(super) fn select_provider_configs_metadata(
    connection: &Connection,
) -> rusqlite::Result<Vec<ConfiguredProvider>> {
    let mut statement = connection.prepare(
        "
        SELECT id, provider_id, provider_name, auth_method, base_url, api_key_label,
               credential_configured, custom_model_ids, auto_route_target,
               created_at_ms, updated_at_ms
        FROM provider_configs
        ORDER BY created_at_ms DESC
        ",
    )?;
    let providers = statement
        .query_map([], |row| {
            Ok(ConfiguredProvider {
                id: row.get(0)?,
                provider_id: row.get(1)?,
                provider_name: row.get(2)?,
                auth_method: row.get(3)?,
                base_url: row.get(4)?,
                api_key_label: row.get(5)?,
                api_key: None,
                credential_configured: row.get::<_, i64>(6)? == 1,
                custom_model_ids: row.get(7)?,
                auto_route_target: row.get::<_, i64>(8)? == 1,
                created_at_ms: row.get(9)?,
                updated_at_ms: row.get(10)?,
            })
        })?
        .collect();
    providers
}

pub(super) fn get_active_auto_route_target(
    connection: &Connection,
) -> rusqlite::Result<Option<ConfiguredProvider>> {
    connection
        .query_row(
            "
            SELECT id, provider_id, provider_name, auth_method, base_url, api_key_label,
                   CASE WHEN credential_configured = 1 OR (api_key IS NOT NULL AND trim(api_key) != '') THEN 1 ELSE 0 END,
                   custom_model_ids, auto_route_target, created_at_ms, updated_at_ms, api_key
            FROM provider_configs
            WHERE auto_route_target = 1
            LIMIT 1
            ",
            [],
            provider_metadata_row,
        )
        .optional()?
        .map(|(mut config, legacy_secret_present)| {
            reconcile_provider_credential_metadata(
                connection,
                &mut config,
                legacy_secret_present,
            )?;
            Ok(config)
        })
        .transpose()
}

fn provider_metadata_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<(ConfiguredProvider, bool)> {
    Ok((
        ConfiguredProvider {
            id: row.get(0)?,
            provider_id: row.get(1)?,
            provider_name: row.get(2)?,
            auth_method: row.get(3)?,
            base_url: row.get(4)?,
            api_key_label: row.get(5)?,
            api_key: None,
            credential_configured: row.get::<_, i64>(6)? == 1,
            custom_model_ids: row.get(7)?,
            auto_route_target: row.get::<_, i64>(8)? == 1,
            created_at_ms: row.get(9)?,
            updated_at_ms: row.get(10)?,
        },
        row.get::<_, Option<String>>(11)?.is_some(),
    ))
}

pub(super) fn select_provider_config_with_secret(
    connection: &Connection,
    id: &str,
) -> rusqlite::Result<Option<ConfiguredProvider>> {
    let row = connection
        .query_row(
            "
            SELECT id, provider_id, provider_name, auth_method, base_url, api_key_label,
                   credential_configured, custom_model_ids, auto_route_target,
                   created_at_ms, updated_at_ms, api_key
            FROM provider_configs
            WHERE id = ?1
            ",
            params![id],
            |row| {
                let config = ConfiguredProvider {
                    id: row.get(0)?,
                    provider_id: row.get(1)?,
                    provider_name: row.get(2)?,
                    auth_method: row.get(3)?,
                    base_url: row.get(4)?,
                    api_key_label: row.get(5)?,
                    api_key: None,
                    credential_configured: row.get::<_, i64>(6)? == 1,
                    custom_model_ids: row.get(7)?,
                    auto_route_target: row.get::<_, i64>(8)? == 1,
                    created_at_ms: row.get(9)?,
                    updated_at_ms: row.get(10)?,
                };
                Ok((config, row.get::<_, Option<String>>(11)?))
            },
        )
        .optional()?;
    row.map(|(config, legacy)| hydrate_selected_provider_secret(connection, config, legacy))
        .transpose()
}
