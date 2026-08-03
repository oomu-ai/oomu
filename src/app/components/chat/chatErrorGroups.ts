const CONTEXTUAL_ERROR_GROUPS: Record<string, string> = {
  contextual_filename_required: "contextual_filename",
  contextual_file_preparation_failed: "contextual_file_preparation",
  contextual_output_name_invalid: "contextual_output",
  delete_target_not_found: "delete_target_not_found",
};

function isPrivateEgressError(errorCode: string) {
  return ["private_egress_", "private_source_", "private_provenance_"]
    .some((prefix) => errorCode.startsWith(prefix));
}

function directErrorGroup(errorCode: string) {
  const contextualGroup = CONTEXTUAL_ERROR_GROUPS[errorCode];
  if (contextualGroup) return contextualGroup;
  if (isPrivateEgressError(errorCode)) return "private_egress";
  return null;
}

export function chatErrorGroup(errorCode: string, detail = "") {
  const normalizedDetail = detail.toLowerCase();
  const directGroup = directErrorGroup(errorCode);
  if (directGroup) return directGroup;
  if (
    errorCode === "planner_connector_binding_mismatch" ||
    errorCode.startsWith("connector_planned_")
  ) {
    return "connector_authority";
  }
  if (
    normalizedDetail.includes("approved external file write failed") ||
    normalizedDetail.includes("safe temporary file") ||
    normalizedDetail.includes("temporary file name is not valid") ||
    normalizedDetail.includes("verify the prepared file") ||
    normalizedDetail.includes("approved file changed before oomu could save it")
  ) {
    return "external_file_write";
  }
  if (
    errorCode.startsWith("classifier_") ||
    errorCode.startsWith("auto_route_") ||
    errorCode === "dynamic_routing_audit_persistence_failed"
  ) {
    return "auto_route_attention";
  }
  if (errorCode === "inference_retry_exhausted") {
    if (normalizedDetail.includes("provider_rate_limited")) {
      return "provider_rate_limited";
    }
    if (
      normalizedDetail.includes("provider_network_error") ||
      normalizedDetail.includes("provider_stream_interrupted_after_tokens")
    ) {
      return "provider_network";
    }
    if (normalizedDetail.includes("provider_response_error")) {
      return "provider_response";
    }
    if (normalizedDetail.includes("local_") || normalizedDetail.includes("llama_")) {
      return "local_helper";
    }
  }
  switch (errorCode) {
    case "local_model_not_found":
    case "local_model_fallback_unavailable":
    case "gemma_asset_missing":
    case "local_model_incompatible":
    case "local_infer_stateful_gguf_required":
      return "model_setup";
    case "gemma_metal_required":
      return "metal";
    case "local_inference_timeout":
      return "timeout";
    case "local_model_repetition_collapse":
      return "repetition";
    case "provider_network_error":
    case "provider_stream_interrupted_after_tokens":
    case "provider_stream_duration_exceeded":
      return "provider_network";
    case "provider_rate_limited":
      return "provider_rate_limited";
    case "credential_unavailable":
      return "credentials";
    case "provider_response_error":
      return "provider_response";
    case "project_provider_blocked":
      return "project_provider_blocked";
    case "project_provider_consent_required":
    case "project_provider_confirmation_invalid":
      return "project_provider_consent";
    case "chat_turn_already_running":
      return "turn_in_progress";
    case "chat_turn_persistence_failed":
      return "turn_persistence";
    case "planner_output_unusable":
    case "planner_prompt_compilation_failed":
    case "dynamic_planner_route_failed":
    case "planner_provider_configuration_failed":
      return "planner_unavailable";
    case "dynamic_planner_cloud_target_unavailable":
      return "credentials";
    case "planner_cloud_model_unavailable":
    case "cloud_planner_failed":
      return "provider_response";
    case "planner_objective_too_large":
      return "planner_too_large";
    case "agent_objective_not_executable":
      return "local_action_unavailable";
    case "file_creation_failed":
      return "file_creation";
    case "mlc_verification_failed":
      return "final_verification";
    case "approved_file_unavailable":
      return "approved_file";
    case "approved_file_attachment_limit":
      return "approved_file_limit";
    case "workspace_boundary_violation":
      return "boundary";
    case "permission_denied":
    case "authority_user_denied":
    case "shield_approval_denied":
      return "permission_denied";
    case "permission_request_failed":
    case "permission_prompt_unavailable":
    case "permission_check_failed":
    case "mcp_permission_required":
    case "shield_approval_not_found":
    case "shield_approval_event_failed":
    case "shield_approval_channel_closed":
    case "shield_approval_timeout":
    case "shield_approval_resume_failed":
    case "authority_native_prompt_failed":
    case "authority_native_prompt_closed":
    case "authority_native_prompt_unavailable":
    case "authority_native_prompt_window_unavailable":
    case "authority_native_prompt_timeout":
    case "shield_native_prompt_failed":
    case "shield_native_prompt_closed":
    case "shield_native_prompt_unavailable":
    case "shield_native_prompt_window_unavailable":
      return "permission_request";
    case "mod_requirement_blocked":
      return "mod_requirement";
    case "ledger_integrity_violation":
    case "memory_identity_quarantined":
    case "sovereign_identity_keyring_unavailable":
    case "identity_secure_storage_error":
    case "identity_invalid_crypto_material":
    case "profile_persistence_failed":
    case "profile_persistence_receipt_missing":
    case "profile_persistence_receipt_invalid":
    case "secure_memory_unavailable":
      return "secure_memory";
    case "local_infer_invalid_response":
    case "local_infer_failed":
    case "local_infer_protocol_timeout":
    case "worker_error":
      return "local_helper";
    default:
      return "default";
  }
}
