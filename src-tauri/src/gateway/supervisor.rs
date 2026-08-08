use super::*;

const GATEWAY_WORKER_POLL_INTERVAL: Duration = Duration::from_secs(5);

impl SovereignGatewayService {
    pub(super) fn spawn_worker_supervisor(&self, persistence: PersistenceEngine) {
        let service = self.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(GATEWAY_WORKER_POLL_INTERVAL).await;
                if service.shutting_down.load(Ordering::Acquire) {
                    break;
                }
                if !service.workers_are_enabled() || !service.supervisor_refresh_due() {
                    continue;
                }
                if let Err(error) = service.refresh_workers(&persistence).await {
                    eprintln!(
                        "SOVEREIGN_GATEWAY_SUPERVISOR_REFRESH_FAILED error={}",
                        compact_log_text(&error, 160)
                    );
                }
            }
        });
    }

    fn supervisor_refresh_due(&self) -> bool {
        let Ok(inner) = self.inner.lock() else {
            return true;
        };
        let has_finished_worker = inner
            .workers
            .values()
            .any(|worker| worker.finished.load(Ordering::Acquire));
        let next_restart_at_ms = inner.worker_restart_after_ms.values().min().copied();
        gateway_supervisor_refresh_due(has_finished_worker, next_restart_at_ms, unix_time_ms())
    }
}

fn gateway_supervisor_refresh_due(
    has_finished_worker: bool,
    next_restart_at_ms: Option<i64>,
    now_ms: i64,
) -> bool {
    has_finished_worker || next_restart_at_ms.is_some_and(|restart_at| restart_at <= now_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_gateway_supervisor_does_not_reopen_storage() {
        assert!(!gateway_supervisor_refresh_due(false, None, 1_000));
        assert!(!gateway_supervisor_refresh_due(false, Some(1_500), 1_000));
        assert!(gateway_supervisor_refresh_due(false, Some(1_000), 1_000));
        assert!(gateway_supervisor_refresh_due(true, None, 1_000));
    }
}
