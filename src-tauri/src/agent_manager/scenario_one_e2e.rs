#![cfg(debug_assertions)]

use super::{unix_time_ms, AgentManager};
use rusqlite::params;

impl AgentManager {
    pub(crate) fn configure_scenario_one_e2e_model(&self) -> Result<usize, String> {
        if !crate::scenario_one_e2e_profile::enabled() {
            return Ok(0);
        }
        let model_root = crate::settings::resolved_local_model_directory_headless();
        crate::gemma::resolve_strict_local_model(
            &model_root,
            crate::scenario_one_e2e_profile::LOCAL_MODEL_ID,
        )
        .map_err(|error| error.message)?;
        let _guard = self.lock_writes();
        let connection = self.open_connection().map_err(|error| error.to_string())?;
        connection
            .execute(
                "
                UPDATE agent_configs
                SET model_id = ?1, updated_at_ms = ?2
                WHERE lower(replace(provider_id, '-', '_')) IN ('local', 'local_model', 'local_gemma')
                ",
                params![
                    crate::scenario_one_e2e_profile::LOCAL_MODEL_ID,
                    unix_time_ms()
                ],
            )
            .map_err(|error| error.to_string())
    }
}
