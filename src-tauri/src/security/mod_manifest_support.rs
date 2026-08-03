use super::mods::ModPermission;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

pub(super) fn manifest_permissions(value: &Value) -> Vec<ModPermission> {
    match value {
        Value::Array(items) => {
            let permissions = items
                .iter()
                .filter_map(|item| {
                    if let Some(label) = item.as_str().map(str::trim).filter(|v| !v.is_empty()) {
                        return Some(ModPermission {
                            label: title_from_key(label),
                            detail: "Permission declared by the mod manifest.".into(),
                        });
                    }
                    let label = item.get("label")?.as_str()?.trim();
                    if label.is_empty() {
                        return None;
                    }
                    Some(ModPermission {
                        label: label.into(),
                        detail: item
                            .get("detail")
                            .and_then(Value::as_str)
                            .unwrap_or("Permission declared by the mod manifest.")
                            .trim()
                            .into(),
                    })
                })
                .collect::<Vec<_>>();
            if permissions.is_empty() {
                default_permissions()
            } else {
                permissions
            }
        }
        Value::Object(map) if map.is_empty() => default_permissions(),
        Value::Object(map) => map
            .iter()
            .map(|(key, detail)| ModPermission {
                label: title_from_key(key),
                detail: permission_detail_from_value(detail),
            })
            .collect(),
        _ => default_permissions(),
    }
}

fn permission_detail_from_value(value: &Value) -> String {
    match value {
        Value::String(detail) => detail.clone(),
        Value::Array(items) => {
            let details = items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|detail| !detail.is_empty())
                .collect::<Vec<_>>();
            if details.is_empty() {
                value.to_string()
            } else {
                details.join(", ")
            }
        }
        _ => value.to_string(),
    }
}

pub(super) fn default_permissions() -> Vec<ModPermission> {
    vec![ModPermission {
        label: "No extra permissions".into(),
        detail: "The manifest does not request additional local permissions.".into(),
    }]
}

pub(super) fn inferred_category(hooks: &Value) -> String {
    if hooks.get("shield_gate").is_some() {
        "Prompt Hook".into()
    } else {
        "Installed".into()
    }
}

fn title_from_key(key: &str) -> String {
    key.replace(['_', '-'], " ")
        .split_whitespace()
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map_or_else(String::new, |first| {
                format!("{}{}", first.to_uppercase(), chars.as_str())
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn valid_mod_identifier(value: &str) -> bool {
    value == value.trim()
        && (3..=255).contains(&value.len())
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'_')
        })
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
}

pub(super) fn storage_id(id: &str) -> String {
    let sanitized = id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches(['.', '_', '-'])
        .to_string();
    if sanitized.is_empty() {
        format!("mod-{}", crate::foundation::clock::unix_time_ms_i64())
    } else {
        sanitized
    }
}

pub(super) fn ensure_no_case_colliding_mod_id(
    connection: &Connection,
    id: &str,
) -> Result<(), String> {
    let collision: Option<String> = connection
        .query_row(
            "SELECT id FROM installed_mods WHERE id<>?1 AND lower(id)=lower(?1) LIMIT 1",
            [id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if collision.is_some() {
        Err("A mod with this identifier is already installed.".to_string())
    } else {
        Ok(())
    }
}

pub(super) fn exact_package_is_revoked(
    connection: &Connection,
    mod_id: &str,
    version: &str,
    payload_sha256: &str,
    manifest: &Value,
    manifest_json: &str,
) -> Result<bool, String> {
    let recorded_revocation: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM capability_bundle_records WHERE mod_id=?1 AND package_version=?2 AND payload_sha256=?3 AND review_state='revoked')",
            params![mod_id, version, payload_sha256],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if recorded_revocation {
        return Ok(true);
    }

    let Some((bundle_id, bundle_version)) = package_bundle_coordinates(manifest, manifest_json)
    else {
        return Ok(false);
    };
    if bundle_version != version {
        return Ok(false);
    }
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM capability_registry_entries WHERE bundle_id=?1 AND package_version=?2 AND metadata_sha256=?3 AND review_state='revoked')",
            params![bundle_id, bundle_version, payload_sha256],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}

fn package_bundle_coordinates(manifest: &Value, manifest_json: &str) -> Option<(String, String)> {
    let version = manifest.get("version")?.as_str()?.trim();
    if let Some(bundle) = manifest.get("capability_bundle") {
        let bundle_id = bundle
            .get("capabilityBundleId")
            .or_else(|| bundle.get("id"))?
            .as_str()?
            .trim();
        let bundle_version = bundle
            .get("packageVersion")
            .or_else(|| bundle.get("version"))?
            .as_str()?
            .trim();
        return (!bundle_id.is_empty() && !bundle_version.is_empty())
            .then(|| (bundle_id.to_string(), bundle_version.to_string()));
    }
    let digest = crate::foundation::digest::sha256_hex(manifest_json.as_bytes());
    Some((
        format!("bundle_legacy_{}", &digest[..32]),
        version.to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_case_variant_cannot_alias_new_mod_storage() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute("CREATE TABLE installed_mods (id TEXT PRIMARY KEY)", [])
            .unwrap();
        connection
            .execute(
                "INSERT INTO installed_mods (id) VALUES ('Com.Acme.Mod')",
                [],
            )
            .unwrap();
        assert!(ensure_no_case_colliding_mod_id(&connection, "com.acme.mod").is_err());
        assert!(ensure_no_case_colliding_mod_id(&connection, "Com.Acme.Mod").is_ok());
    }
}
