use super::{
    has_matching_approved_file_attachment, workspace_data_resource_for_private_app_kind,
    workspace_data_resources_for_attachments, ChatAttachment, ConversationalMcpToolCapability,
};

pub(super) fn requires_agentic_escalation(
    decision: &crate::agentic_loop::ChatIntentRouteDecision,
    _user_message: &str,
    capabilities: &[ConversationalMcpToolCapability],
) -> bool {
    matches!(
        decision.route,
        crate::agentic_loop::ChatIntentRoute::AgenticPlanner
    ) && (matches!(
        decision.decision_source.as_str(),
        "native_artifact_creation_filter" | "deterministic_decision_pack_filter"
    ) || !capabilities.iter().any(|capability| {
        !capability.server_name.trim().is_empty() && !capability.tool_name.trim().is_empty()
    }))
}

pub(super) fn filter_conversational_mcp_tool_capabilities_for_turn(
    capabilities: &[ConversationalMcpToolCapability],
    attachments: &[ChatAttachment],
    _route_decision: &crate::agentic_loop::ChatIntentRouteDecision,
    _prompt: &str,
) -> Vec<ConversationalMcpToolCapability> {
    // Capability exposure is not execution authority. Preserve every connected
    // catalog entry so the model can select tools across languages and natural
    // phrasing; the native broker still validates the exact tool, arguments,
    // current session, permission state, and any required approval at execution.
    // Only suppress a read when the same bounded native result is already
    // attached to this turn, which prevents duplicate private-data access.
    let resources = workspace_data_resources_for_attachments(attachments);
    capabilities
        .iter()
        .filter(|capability| {
            !super::workspace_data_attachment_blocks_tool(
                &resources,
                &capability.server_name,
                &capability.tool_name,
            )
        })
        .cloned()
        .collect()
}

pub(super) fn enforce_backend_executable_intent_gate(
    decision: crate::agentic_loop::ChatIntentRouteDecision,
    user_message: &str,
    attachments: &[ChatAttachment],
) -> crate::agentic_loop::ChatIntentRouteDecision {
    let planner_route = matches!(
        decision.route,
        crate::agentic_loop::ChatIntentRoute::AgenticPlanner
    );
    let workspace_resources = workspace_data_resources_for_attachments(attachments);
    let requested_app_kind = crate::local_app_intent::private_app_data_kind(user_message);
    let requested_resource =
        requested_app_kind.and_then(workspace_data_resource_for_private_app_kind);
    let normalized_message = user_message.trim().to_ascii_lowercase();
    let approved_file_marker_present =
        crate::agentic_loop::contains_approved_file_marker(user_message);
    let matching_approved_file_context =
        has_matching_approved_file_attachment(user_message, attachments);
    if planner_route && approved_file_marker_present && !matching_approved_file_context {
        return crate::agentic_loop::ChatIntentRouteDecision {
            route: crate::agentic_loop::ChatIntentRoute::ConversationalStream,
            requires_local_access: false,
            decision_source: "approved_file_context_missing_filter".to_string(),
            reason: "An approved-file label without its exact bounded content grants no file access and cannot enter action planning."
                .to_string(),
            matched_signals: vec!["approved file label without bounded context".to_string()],
            status_label: "OOMU is typing...".to_string(),
        };
    }
    let direct_private_app_read = planner_route
        && requested_resource.is_some()
        && crate::agentic_loop::is_direct_private_app_read_objective(
            user_message,
            &normalized_message,
        )
        && requested_app_kind.is_some_and(|app_kind| {
            crate::local_app_intent::is_focused_local_app_shortcut_request(user_message, app_kind)
        });
    let hydrated_read_only_result = direct_private_app_read
        && requested_resource.is_some_and(|resource| workspace_resources.contains(&resource));
    let hydrated_local_file_read = planner_route
        && requested_resource.is_none()
        && (!approved_file_marker_present || matching_approved_file_context)
        && attachments.iter().any(|attachment| {
            attachment
                .text
                .as_deref()
                .is_some_and(|text| !text.trim().is_empty())
        })
        && crate::agentic_loop::is_read_only_local_context_request(user_message);
    if hydrated_read_only_result {
        eprintln!(
            "CHAT_ROUTE_HYDRATED_RESULT_FALLBACK prior_source={} attachment_count={} message_chars={}",
            decision.decision_source,
            attachments.len(),
            user_message.chars().count()
        );
        return crate::agentic_loop::ChatIntentRouteDecision {
            route: crate::agentic_loop::ChatIntentRoute::ConversationalStream,
            requires_local_access: false,
            decision_source: "backend_hydrated_workspace_data_gate".to_string(),
            reason: "A bounded native read result is already attached, so this turn may summarize it but cannot re-enter action planning."
                .to_string(),
            matched_signals: vec!["hydrated workspace data".to_string()],
            status_label: "OOMU is reading the local result...".to_string(),
        };
    }
    if hydrated_local_file_read {
        eprintln!(
            "CHAT_ROUTE_HYDRATED_FILE_FALLBACK prior_source={} attachment_count={} message_chars={}",
            decision.decision_source,
            attachments.len(),
            user_message.chars().count()
        );
        return crate::agentic_loop::ChatIntentRouteDecision {
            route: crate::agentic_loop::ChatIntentRoute::ConversationalStream,
            requires_local_access: false,
            decision_source: "backend_hydrated_local_context_gate".to_string(),
            reason: "Bounded local file content is already attached, so this read-only turn cannot re-enter action planning."
                .to_string(),
            matched_signals: vec!["hydrated local file context".to_string()],
            status_label: "OOMU is reading the approved file...".to_string(),
        };
    }
    if planner_route
        && !direct_private_app_read
        && (crate::local_app_intent::is_informational_local_app_question(user_message)
            || !crate::agentic_loop::has_executable_agent_objective(user_message))
    {
        eprintln!(
            "CHAT_ROUTE_EXECUTABLE_GATE_FALLBACK prior_source={} message_chars={}",
            decision.decision_source,
            user_message.chars().count()
        );
        return crate::agentic_loop::ChatIntentRouteDecision {
            route: crate::agentic_loop::ChatIntentRoute::ConversationalStream,
            requires_local_access: false,
            decision_source: "backend_executable_intent_gate".to_string(),
            reason: "The backend found no executable user objective, so this turn remains conversational."
                .to_string(),
            matched_signals: Vec::new(),
            status_label: "OOMU is typing...".to_string(),
        };
    }
    decision
}
