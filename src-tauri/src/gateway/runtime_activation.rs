use super::*;

impl SovereignGatewayService {
    pub fn attach_app_handle(&self, app_handle: tauri::AppHandle) -> Result<(), String> {
        {
            let mut stored = self
                .app_handle
                .lock()
                .map_err(|_| "Gateway app handle lock poisoned.".to_string())?;
            *stored = Some(app_handle);
        }
        if let Ok(mut inner) = self.inner.lock() {
            inner.app_handle = self
                .app_handle
                .lock()
                .ok()
                .and_then(|stored| stored.clone());
        }
        eprintln!("SOVEREIGN_GATEWAY_RUNTIME_ATTACHED");
        Ok(())
    }

    pub(super) fn workers_are_enabled(&self) -> bool {
        self.workers_enabled.load(Ordering::Acquire)
    }

    pub(super) fn enable_workers_for_explicit_connection_action(&self) {
        self.workers_enabled.store(true, Ordering::Release);
    }
}

#[tauri::command]
pub(crate) async fn get_channel_statuses(
    gateway: tauri::State<'_, SovereignGatewayService>,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Vec<GatewayChannelStatus>, String> {
    gateway.enable_workers_for_explicit_connection_action();
    gateway.snapshot_statuses(persistence.inner()).await
}
