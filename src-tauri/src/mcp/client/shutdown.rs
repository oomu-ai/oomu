use super::*;
use std::time::Duration;

impl McpClientSession {
    pub async fn shutdown(&self) -> Result<(), McpClientError> {
        self.remote_cancellation.store(true, Ordering::Release);
        let child = self
            .child_handle
            .lock()
            .map_err(|_| {
                McpClientError::transport(format!(
                    "Failed to lock MCP child handle for '{}'.",
                    self.server_name
                ))
            })?
            .take();
        if let Some(mut child) = child {
            child.kill().await.map_err(|error| {
                McpClientError::transport(format!(
                    "Failed to terminate MCP server '{}': {error}",
                    self.server_name
                ))
            })?;
        }
        Ok(())
    }
}

impl McpClientRegistry {
    pub(crate) async fn shutdown_all(&self) -> Result<(), McpClientError> {
        self.accepting_work.store(false, Ordering::Release);
        let _lifecycle = self.connection_lifecycle.lock().await;
        let connecting = self
            .connecting_remote
            .lock()
            .await
            .drain()
            .map(|(_, cancellation)| cancellation)
            .collect::<Vec<_>>();
        for cancellation in connecting {
            cancellation.store(true, Ordering::Release);
        }
        let sessions = self
            .sessions
            .lock()
            .await
            .drain()
            .map(|(_, session)| session)
            .collect::<Vec<_>>();
        self.tool_catalog.lock().await.clear();
        self.pending_tool_approvals.lock().await.clear();
        self.public_search_chat_session_grants.lock().await.clear();
        self.spawn_authorizations.lock().await.clear();
        let mut first_failure = None;
        for session in sessions {
            if let Err(error) = session.shutdown().await {
                first_failure.get_or_insert(error);
            }
        }
        first_failure.map_or(Ok(()), Err)
    }

    pub(crate) fn shutdown_all_blocking(&self) -> Result<(), String> {
        tauri::async_runtime::block_on(async {
            tokio::time::timeout(Duration::from_secs(5), self.shutdown_all())
                .await
                .map_err(|_| "mcp_registry_shutdown_timeout".to_string())?
                .map_err(|error| error.code.to_string())
        })
    }
}

impl Drop for McpClientSession {
    fn drop(&mut self) {
        self.remote_cancellation.store(true, Ordering::Release);
        if let Ok(mut child_handle) = self.child_handle.lock() {
            if let Some(mut child) = child_handle.take() {
                let _ = child.start_kill();
            }
        }
    }
}
