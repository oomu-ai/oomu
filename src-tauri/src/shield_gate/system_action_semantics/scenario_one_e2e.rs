#[cfg(debug_assertions)]
use super::super::ShieldApprovalRequest;

#[cfg(debug_assertions)]
fn research_policy_matches(request: &ShieldApprovalRequest) -> bool {
    let Some(policy) = serde_json::from_str::<serde_json::Value>(&request.preview)
        .ok()
        .and_then(|preview| preview.get("researchPolicy").cloned())
        .and_then(|policy| {
            serde_json::from_value::<crate::decision_research_policy::ResearchPolicy>(policy).ok()
        })
    else {
        return false;
    };
    crate::decision_research_policy::validate_research_policy(&policy).is_ok()
        && crate::decision_research_policy::compile_research_policy(
            "independently research current primary or official web sources for fuel or freight conditions",
        )
        .is_ok_and(|expected| policy == expected)
}

#[cfg(debug_assertions)]
fn mail_payload_matches(request: &ShieldApprovalRequest) -> bool {
    let Ok(preview) = serde_json::from_str::<serde_json::Value>(&request.preview) else {
        return false;
    };
    let expected_paths = crate::scenario_one_e2e_profile::output_paths()
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    crate::tools::decision_pack_mail::resolved_preview_matches_exact_contract(
        &preview,
        "recipient@example.com",
        "Supplier Decision Review",
        &expected_paths,
    )
}

#[cfg(debug_assertions)]
pub(super) fn native_approval(
    request: &ShieldApprovalRequest,
) -> Option<crate::scenario_one_e2e_profile::NativeApprovalAutomation> {
    let probe = crate::scenario_one_e2e_profile::NativeApprovalProbe {
        approval_token: &request.approval_token,
        session_id: request.session_id.as_deref(),
        turn_id: request.turn_id.as_deref(),
        generation_token: request.generation_token.as_deref(),
        action_type: &request.action_type,
        action_label: &request.action_label,
        target_path: request.target_path.as_deref(),
        principal: request.principal.as_deref(),
        risk_tier: &request.risk_tier,
        reason: &request.reason,
        estimated_token_costs: request.estimated_token_costs,
        requested_at_ms: request.requested_at_ms,
        preview: &request.preview,
        semantic_summary: &request.semantic_summary,
        semantic_detail: &request.semantic_detail,
        approval_tier: &request.approval_tier,
        approval_mode: &request.approval_mode,
        diff_preview_present: request.diff_preview.is_some(),
        scope_trust_available: request.scope_trust_available,
        scope_trust_prefix: request.scope_trust_prefix.as_deref(),
        scope_trust_duration_ms: request.scope_trust_duration_ms,
        project_id: request.project_id.as_deref(),
        task_run_id: request.task_run_id.as_deref(),
        action_class: &request.action_class,
        argument_class: &request.argument_class,
        canonical_resource: request.canonical_resource.as_deref(),
        mandatory_reconfirm: request.mandatory_reconfirm,
        approval_scope_kinds: &request.approval_scope_kinds,
        research_policy_matches: research_policy_matches(request),
        mail_payload_matches: mail_payload_matches(request),
        calendar_argument_class_matches: request.argument_class
            == crate::approval_scopes::argument_class("calendar_create", "OOMU Test"),
    };
    crate::scenario_one_e2e_profile::automated_native_approval(&probe)
}

#[cfg(not(debug_assertions))]
pub(super) fn native_approval(
    _request: &super::super::ShieldApprovalRequest,
) -> Option<crate::scenario_one_e2e_profile::NativeApprovalAutomation> {
    None
}

pub(in crate::shield_gate) async fn request_native_selection(
    app: &tauri::AppHandle,
    request: &super::super::ShieldApprovalRequest,
    locale: &str,
) -> Result<
    crate::authority::shield_decision::NativeShieldPromptSelection,
    super::super::ShieldGateError,
> {
    let selection = tokio::time::timeout(
        std::time::Duration::from_secs(600),
        crate::authority::shield_decision::request_native_selection(
            app,
            request,
            locale,
            native_approval(request),
        ),
    )
    .await
    .map_err(|_| super::super::ShieldGateError {
        code: "shield_approval_timeout",
        boundary: "ShieldApprovalManager",
        message: "Shield Gate approval expired before a native decision arrived.".to_string(),
    })??;
    Ok(selection)
}
