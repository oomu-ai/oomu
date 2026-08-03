use super::{provider_store, AgentManager, ConfiguredProvider};

impl AgentManager {
    /// Reads routing metadata without hydrating a Keychain secret. Callers that
    /// need a stable identity snapshot must hold `lock_writes` for the complete
    /// validation-and-commit boundary.
    pub(crate) fn select_provider_configs_metadata_locked(
        &self,
    ) -> rusqlite::Result<Vec<ConfiguredProvider>> {
        let connection = self.open_connection()?;
        provider_store::select_provider_configs_metadata(&connection)
    }

    pub(crate) fn select_provider_configs_metadata(
        &self,
    ) -> rusqlite::Result<Vec<ConfiguredProvider>> {
        let _guard = self.lock_writes();
        self.select_provider_configs_metadata_locked()
    }

    pub(crate) fn select_provider_config_locked(
        &self,
        id: &str,
    ) -> rusqlite::Result<Option<ConfiguredProvider>> {
        let connection = self.open_connection()?;
        provider_store::select_provider_config_with_secret(&connection, id)
    }
}
