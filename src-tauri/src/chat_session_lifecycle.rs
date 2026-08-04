use crate::agentic_loop::AgenticLoopError;
use crate::db::PersistenceEngine;
use crate::mcp::client::McpClientRegistry;

trait ChatSessionAuthorityRevoker {
    async fn revoke_chat_session_authority(&self, session_id: &str) -> usize;
}

impl ChatSessionAuthorityRevoker for McpClientRegistry {
    async fn revoke_chat_session_authority(&self, session_id: &str) -> usize {
        self.revoke_public_search_chat_session_authority(session_id)
            .await
    }
}

async fn revoke_authority_after_persisted_change(
    changed: bool,
    session_id: &str,
    revoker: &impl ChatSessionAuthorityRevoker,
) {
    if changed {
        revoker.revoke_chat_session_authority(session_id).await;
    }
}

#[tauri::command]
pub async fn delete_chat_session(
    session_id: String,
    persistence: tauri::State<'_, PersistenceEngine>,
    mcp_registry: tauri::State<'_, McpClientRegistry>,
) -> Result<bool, AgenticLoopError> {
    let engine = persistence.inner().clone();
    let deletion_session_id = session_id.clone();
    let deleted = tauri::async_runtime::spawn_blocking(move || {
        engine.delete_chat_session_by_id(&deletion_session_id)
    })
    .await
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?;
    revoke_authority_after_persisted_change(deleted, &session_id, mcp_registry.inner()).await;
    Ok(deleted)
}

#[tauri::command]
pub async fn stage_chat_session_deletion(
    session_id: String,
    persistence: tauri::State<'_, PersistenceEngine>,
    mcp_registry: tauri::State<'_, McpClientRegistry>,
) -> Result<bool, AgenticLoopError> {
    let engine = persistence.inner().clone();
    let deletion_session_id = session_id.clone();
    let staged = tauri::async_runtime::spawn_blocking(move || {
        engine.stage_chat_session_deletion_by_id(&deletion_session_id)
    })
    .await
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?;
    revoke_authority_after_persisted_change(staged, &session_id, mcp_registry.inner()).await;
    Ok(staged)
}

#[tauri::command]
pub async fn commit_chat_session_deletion(
    session_id: String,
    persistence: tauri::State<'_, PersistenceEngine>,
    mcp_registry: tauri::State<'_, McpClientRegistry>,
) -> Result<bool, AgenticLoopError> {
    let engine = persistence.inner().clone();
    let deletion_session_id = session_id.clone();
    let committed = tauri::async_runtime::spawn_blocking(move || {
        engine.commit_chat_session_deletion_by_id(&deletion_session_id)
    })
    .await
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?;
    revoke_authority_after_persisted_change(committed, &session_id, mcp_registry.inner()).await;
    Ok(committed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingRevoker {
        revoked_session_ids: Mutex<Vec<String>>,
    }

    impl ChatSessionAuthorityRevoker for RecordingRevoker {
        async fn revoke_chat_session_authority(&self, session_id: &str) -> usize {
            self.revoked_session_ids
                .lock()
                .expect("revocation recorder remains available")
                .push(session_id.to_string());
            1
        }
    }

    #[tokio::test]
    async fn revokes_only_after_a_persisted_chat_session_change() {
        let revoker = RecordingRevoker::default();

        revoke_authority_after_persisted_change(false, "session-preserved", &revoker).await;
        revoke_authority_after_persisted_change(true, "session-deleted", &revoker).await;

        assert_eq!(
            *revoker
                .revoked_session_ids
                .lock()
                .expect("revocation recorder remains available"),
            vec!["session-deleted"]
        );
    }
}
