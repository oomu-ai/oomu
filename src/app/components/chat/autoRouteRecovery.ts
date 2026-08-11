import { stableErrorCode } from "./inferenceErrors";

export type AutoRouteRecoveryKind =
  | "choose_model"
  | "preparing"
  | "timeout"
  | "cloud_setup"
  | "saved_work_check"
  | "interrupted"
  | "unknown";

const safeEvidenceToken = /^[a-z][a-z0-9_]{0,95}$/;

function stableErrorBoundary(error: unknown) {
  if (typeof error === "string") {
    try {
      return stableErrorBoundary(JSON.parse(error));
    } catch {
      return null;
    }
  }
  if (!error || typeof error !== "object") return null;
  const candidate = error as { boundary?: unknown; errorBoundary?: unknown };
  const boundary = typeof candidate.boundary === "string"
    ? candidate.boundary
    : typeof candidate.errorBoundary === "string"
      ? candidate.errorBoundary
      : "";
  return safeEvidenceToken.test(boundary) ? boundary : null;
}

export function autoRouteRecoveryKindForCode(codeValue: string): AutoRouteRecoveryKind {
  const code = codeValue.trim().toLowerCase();
  if (code === "local_inference_cancelled" || code === "turn_interrupted") {
    return "interrupted";
  }
  if (
    code.includes("baseline")
    || code === "auto_route_session_binding_invalid"
    || code === "auto_route_session_missing"
    || code === "auto_route_session_local_model_unavailable"
    || code === "auto_route_session_context_invalid"
    || code === "classifier_model_not_configured"
    || code === "classifier_model_ambiguous"
    || code === "classifier_model_unavailable"
  ) {
    return "choose_model";
  }
  if (
    code === "auto_route_cloud_target_missing"
    || code === "auto_route_cloud_model_missing"
    || code === "auto_route_cloud_target_lookup_failed"
    || code === "auto_route_cloud_credential_missing"
  ) {
    return "cloud_setup";
  }
  if (
    code.includes("audit_persistence")
    || code.includes("policy_persistence")
    || code.includes("storage_recovery")
    || code === "auto_route_turn_continuation_invalid"
    || code === "auto_route_turn_persistence_failed"
  ) {
    return "saved_work_check";
  }
  if (code.includes("timeout") || code.includes("deadline")) {
    return "timeout";
  }
  if (
    code.includes("preparing")
    || code.includes("recovering")
    || code.includes("loading")
    || code === "classifier_cold"
    || code === "classifier_not_ready"
    || code === "auto_route_classifier_not_ready"
    || code === "auto_route_classifier_assignment_changed"
  ) {
    return "preparing";
  }
  return "unknown";
}

export function autoRouteRecoveryEvidence(error: unknown) {
  const unsafeCode = stableErrorCode(error);
  const code = safeEvidenceToken.test(unsafeCode) ? unsafeCode : "auto_route_unavailable";
  return {
    code,
    boundary: stableErrorBoundary(error),
    kind: autoRouteRecoveryKindForCode(code),
  };
}
