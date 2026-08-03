use crate::foundation::digest::sha256_hex;

fn account(scope: &str, identifier: &str) -> String {
    format!("{scope}-{}", sha256_hex(identifier.as_bytes()))
}

fn set(scope: &str, identifier: &str, secret: &str) -> Result<(), String> {
    crate::keychain_session::set_password(
        crate::keychain_namespace::backend_credentials_service(),
        &account(scope, identifier),
        secret,
    )
    .map_err(|_| "credential_store_write_failed".to_string())
}

fn get(scope: &str, identifier: &str) -> Result<Option<String>, String> {
    crate::keychain_session::get_password(
        crate::keychain_namespace::backend_credentials_service(),
        &account(scope, identifier),
    )
    .map_err(|_| "credential_store_read_failed".to_string())
}

fn delete(scope: &str, identifier: &str) -> Result<(), String> {
    crate::keychain_session::delete_password(
        crate::keychain_namespace::backend_credentials_service(),
        &account(scope, identifier),
    )
    .map_err(|_| "credential_store_delete_failed".to_string())
}

pub fn set_provider_secret(provider_config_id: &str, secret: &str) -> Result<(), String> {
    set("provider", provider_config_id, secret)
}

pub fn get_provider_secret(provider_config_id: &str) -> Result<Option<String>, String> {
    get("provider", provider_config_id)
}

pub fn provider_secret_exists(provider_config_id: &str) -> Result<bool, String> {
    crate::keychain_session::password_exists(
        crate::keychain_namespace::backend_credentials_service(),
        &account("provider", provider_config_id),
    )
    .map_err(|_| "credential_store_status_failed".to_string())
}

pub fn delete_provider_secret(provider_config_id: &str) -> Result<(), String> {
    delete("provider", provider_config_id)
}

pub fn set_channel_secrets(platform: &str, secrets_json: &str) -> Result<(), String> {
    set("channel", platform, secrets_json)
}

pub fn get_channel_secrets(platform: &str) -> Result<Option<String>, String> {
    get("channel", platform)
}

pub fn delete_channel_secrets(platform: &str) -> Result<(), String> {
    delete("channel", platform)
}

pub fn set_connector_credentials(connector_id: &str, credentials_json: &str) -> Result<(), String> {
    set("connector", connector_id, credentials_json)
}

pub fn get_connector_credentials(connector_id: &str) -> Result<Option<String>, String> {
    get("connector", connector_id)
}

pub fn delete_connector_credentials(connector_id: &str) -> Result<(), String> {
    delete("connector", connector_id)
}

pub fn set_connector_oauth_attempt(attempt_id: &str, secret_json: &str) -> Result<(), String> {
    set("connector-oauth", attempt_id, secret_json)
}

pub fn delete_connector_oauth_attempt(attempt_id: &str) -> Result<(), String> {
    delete("connector-oauth", attempt_id)
}

pub fn set_routine_approval(code_hash: &str, secret_json: &str) -> Result<(), String> {
    set("routine-approval", code_hash, secret_json)
}
pub fn get_routine_approval(code_hash: &str) -> Result<Option<String>, String> {
    get("routine-approval", code_hash)
}
pub fn delete_routine_approval(code_hash: &str) -> Result<(), String> {
    delete("routine-approval", code_hash)
}

#[cfg(test)]
pub(crate) fn evict_provider_secret_for_test(provider_config_id: &str) {
    crate::keychain_session::evict_for_test(
        crate::keychain_namespace::backend_credentials_service(),
        &account("provider", provider_config_id),
    );
}

#[cfg(test)]
pub(crate) fn provider_secret_backend_reads_for_test(provider_config_id: &str) -> usize {
    crate::keychain_session::backend_read_count_for_test(
        crate::keychain_namespace::backend_credentials_service(),
        &account("provider", provider_config_id),
    )
}

#[cfg(test)]
pub(crate) fn remove_provider_secret_backend_value_for_test(provider_config_id: &str) {
    crate::keychain_session::remove_backend_value_for_test(
        crate::keychain_namespace::backend_credentials_service(),
        &account("provider", provider_config_id),
    );
}

#[cfg(test)]
pub(crate) fn evict_channel_secret_for_test(platform: &str) {
    crate::keychain_session::evict_for_test(
        crate::keychain_namespace::backend_credentials_service(),
        &account("channel", platform),
    );
}

#[cfg(test)]
pub(crate) fn channel_secret_backend_reads_for_test(platform: &str) -> usize {
    crate::keychain_session::backend_read_count_for_test(
        crate::keychain_namespace::backend_credentials_service(),
        &account("channel", platform),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_accounts_round_trip_without_exposing_identifiers() {
        let identifier = "provider-secret-canary";
        let account_name = account("provider", identifier);
        assert!(!account_name.contains(identifier));
        set_provider_secret(identifier, "top-secret").unwrap();
        assert_eq!(
            get_provider_secret(identifier).unwrap().as_deref(),
            Some("top-secret")
        );
        delete_provider_secret(identifier).unwrap();
        assert_eq!(get_provider_secret(identifier).unwrap(), None);
    }

    #[test]
    fn repeated_provider_secret_reads_touch_the_backend_once() {
        let identifier = "provider-session-cache-regression";
        set_provider_secret(identifier, "cached-secret").unwrap();
        evict_provider_secret_for_test(identifier);
        let reads_before = provider_secret_backend_reads_for_test(identifier);

        assert_eq!(
            get_provider_secret(identifier).unwrap().as_deref(),
            Some("cached-secret")
        );
        assert_eq!(
            get_provider_secret(identifier).unwrap().as_deref(),
            Some("cached-secret")
        );
        assert_eq!(
            provider_secret_backend_reads_for_test(identifier) - reads_before,
            1
        );
    }
}
