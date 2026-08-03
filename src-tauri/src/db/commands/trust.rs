use super::super::*;

#[tauri::command]
pub async fn upsert_sovereign_trust_policy(
    request: UpsertSovereignTrustPolicyRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    authority: tauri::State<'_, crate::authority::NativeAuthorityManager>,
    identity: tauri::State<'_, crate::sovereign_identity::SovereignIdentity>,
) -> Result<SovereignTrustPolicyResponse, AgenticLoopError> {
    let categories = parse_trust_categories(&request.allowed_tool_categories)
        .map_err(AgenticLoopError::from_persistence)?;
    let permission_level = SovereignTrustPermissionLevel::from_request(&request.permission_level)
        .map_err(AgenticLoopError::from_persistence)?;
    let actor_id = crate::authority::current_actor_id(identity.inner()).map_err(authority_error)?;
    let canonical_scope =
        crate::authority::canonical_scope(&request.directory_path).map_err(authority_error)?;
    authority
        .consume(
            &request.authority_proof_id,
            crate::authority::NativeAuthorityExpectation {
                actor_id,
                session_id: request.session_id.trim().to_string(),
                operation_classes: request.allowed_tool_categories.clone(),
                canonical_scopes: vec![canonical_scope],
                max_steps: 1,
                allowed_persistences: vec![request.permission_level.clone()],
            },
        )
        .map_err(authority_error)?;
    let engine = persistence.inner().clone();
    let policy_id = tauri::async_runtime::spawn_blocking(move || {
        engine.upsert_sovereign_trust_policy(
            &request.directory_path,
            &categories,
            permission_level,
            request.expires_at_ms,
            request.daily_token_cost_limit,
            request.daily_cpu_seconds_limit,
        )
    })
    .await
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?;

    Ok(SovereignTrustPolicyResponse {
        policy_id,
        message: "Sovereign trust policy saved.".to_string(),
    })
}

#[tauri::command]
pub async fn activate_sovereign_trust_session(
    request: ActivateSovereignTrustSessionRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    authority: tauri::State<'_, crate::authority::NativeAuthorityManager>,
    identity: tauri::State<'_, crate::sovereign_identity::SovereignIdentity>,
) -> Result<SovereignTrustSessionResponse, AgenticLoopError> {
    let categories = parse_trust_categories(&request.allowed_tool_categories)
        .map_err(AgenticLoopError::from_persistence)?;
    let actor_id = crate::authority::current_actor_id(identity.inner()).map_err(authority_error)?;
    let canonical_scope =
        crate::authority::canonical_scope(&request.directory_path).map_err(authority_error)?;
    authority
        .consume(
            &request.authority_proof_id,
            crate::authority::NativeAuthorityExpectation {
                actor_id,
                session_id: request.session_id.trim().to_string(),
                operation_classes: request.allowed_tool_categories.clone(),
                canonical_scopes: vec![canonical_scope],
                max_steps: 1,
                allowed_persistences: vec!["session_gated".to_string()],
            },
        )
        .map_err(authority_error)?;
    let expires_at_ms = request
        .expires_at_ms
        .unwrap_or_else(|| unix_time_ms() + SOVEREIGN_TRUST_SESSION_DURATION_MS);
    let engine = persistence.inner().clone();
    let active_session_id = tauri::async_runtime::spawn_blocking(move || {
        engine.activate_sovereign_trust_session(
            &request.session_id,
            &request.directory_path,
            &categories,
            Some(expires_at_ms),
            request.daily_token_cost_limit,
            request.daily_cpu_seconds_limit,
        )
    })
    .await
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?;

    Ok(SovereignTrustSessionResponse {
        active_session_id,
        expires_at_ms,
        message: "Sovereign trust session activated.".to_string(),
    })
}

fn authority_error(error: crate::authority::NativeAuthorityError) -> AgenticLoopError {
    AgenticLoopError {
        code: error.code,
        boundary: error.boundary,
        message: error.message,
        mlc_path: None,
    }
}

#[tauri::command]
pub async fn get_sovereign_trust_dashboard(
    audit_limit: Option<usize>,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<SovereignTrustDashboardResponse, AgenticLoopError> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        engine.select_sovereign_trust_dashboard(audit_limit.unwrap_or(40))
    })
    .await
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
}

#[tauri::command]
pub async fn revoke_sovereign_trust_policy(
    policy_id: i64,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<SovereignTrustMutationResponse, AgenticLoopError> {
    let engine = persistence.inner().clone();
    let affected_rows = tauri::async_runtime::spawn_blocking(move || {
        engine.revoke_sovereign_trust_policy(policy_id)
    })
    .await
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?;

    Ok(SovereignTrustMutationResponse {
        affected_rows,
        message: "Sovereign trust policy revoked.".to_string(),
    })
}

#[tauri::command]
pub async fn revoke_sovereign_trust_session(
    active_session_id: String,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<SovereignTrustMutationResponse, AgenticLoopError> {
    let engine = persistence.inner().clone();
    let affected_rows = tauri::async_runtime::spawn_blocking(move || {
        engine.revoke_sovereign_trust_session(&active_session_id)
    })
    .await
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?;

    Ok(SovereignTrustMutationResponse {
        affected_rows,
        message: "Sovereign trust session revoked.".to_string(),
    })
}
