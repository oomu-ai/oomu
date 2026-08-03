use super::*;

pub(super) fn claim_latest_dropped_local_context_with_store(
    store: &LocalContextGrantStore,
    request: ClaimLatestDroppedLocalContextRequest,
) -> Result<ChooseLocalContextResponse, String> {
    validate_scope(&request.session_id, &request.turn_id)?;
    let paths = {
        let now = unix_time_ms();
        let mut state = store
            .state
            .lock()
            .map_err(|_| "local_context_grant_store_unavailable".to_string())?;
        state
            .pending_drops
            .retain(|_, pending| pending.expires_at_ms > now);
        let drop_id = state
            .pending_drops
            .iter()
            .max_by_key(|(_, pending)| pending.sequence)
            .map(|(drop_id, _)| drop_id.clone())
            .ok_or_else(|| "local_context_drop_invalid_or_expired".to_string())?;
        state
            .pending_drops
            .remove(&drop_id)
            .expect("selected pending drop remains under the same lock")
            .paths
    };
    Ok(issue_grants_for_paths(
        store,
        paths,
        GrantOperation::Read,
        &request.session_id,
        &request.turn_id,
    ))
}
