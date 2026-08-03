use super::*;

impl CommandStatus {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

impl ActuationLeaseManager {
    pub(crate) fn refresh_for_continuation(
        &self,
        app: Option<&tauri::AppHandle>,
        actor_id: &str,
        session_id: &str,
        duration_ms: u64,
    ) -> Result<ActuationLeaseStatus, ShieldGateError> {
        if duration_ms == 0 || duration_ms > 15 * 60 * 1_000 {
            return Err(ShieldGateError {
                code: "actuation_lease_invalid_duration",
                boundary: "ActuationLeaseManager",
                message: "Actuation access cannot exceed 15 minutes.".to_string(),
            });
        }
        let session_id = required_actuation_session_id(Some(session_id))?;
        let actor_id = actor_id.trim();
        let now_ms = unix_time_ms_u64();
        let mut active_lease = self
            .lease
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let lease = active_lease.as_mut().ok_or_else(|| ShieldGateError {
            code: "actuation_lease_missing",
            boundary: "ActuationLeaseManager",
            message: "No active actuation lease is available for this continuation.".to_string(),
        })?;
        if actor_id.is_empty()
            || lease.actor_id != actor_id
            || lease.session_id != session_id
            || lease.canonical_scopes != vec![format!("actuation-session:{session_id}")]
            || lease.current_steps >= lease.max_steps
        {
            return Err(ShieldGateError {
                code: "actuation_lease_continuation_mismatch",
                boundary: "ActuationLeaseManager",
                message: "The active actuation lease does not cover this continuation.".to_string(),
            });
        }
        // Refresh only at bounded continuation points. Scope and remaining budget are immutable.
        lease.expires_at_ms = now_ms.saturating_add(duration_ms);
        lease.is_active = true;
        let status = status_from_lease(Some(lease), now_ms, Some("continued".to_string()));
        if let Some(app) = app {
            let _ = app.emit(ACTUATION_LEASE_UPDATED_EVENT, &status);
        }
        Ok(status)
    }
}
