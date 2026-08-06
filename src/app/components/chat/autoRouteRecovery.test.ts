import { describe, expect, it } from "vitest";
import {
  autoRouteRecoveryEvidence,
  autoRouteRecoveryKindForCode,
} from "./autoRouteRecovery";

describe("cause-specific Auto-route recovery", () => {
  it.each([
    ["auto_route_session_baseline_missing", "choose_model"],
    ["classifier_model_ambiguous", "choose_model"],
    ["auto_route_session_local_model_unavailable", "choose_model"],
    ["auto_route_session_context_invalid", "choose_model"],
    ["classifier_recovering", "preparing"],
    ["auto_route_classifier_not_ready", "preparing"],
    ["auto_route_classifier_assignment_changed", "preparing"],
    ["classifier_inference_timeout", "timeout"],
    ["auto_route_cloud_target_missing", "cloud_setup"],
    ["dynamic_routing_audit_persistence_failed", "saved_work_check"],
    ["local_inference_cancelled", "interrupted"],
    ["classifier_output_invalid", "unknown"],
  ] as const)("maps %s to %s", (code, kind) => {
    expect(autoRouteRecoveryKindForCode(code)).toBe(kind);
  });

  it("retains only stable privacy-safe support evidence", () => {
    expect(autoRouteRecoveryEvidence({
      code: "classifier_inference_timeout",
      boundary: "auto_route_classifier_inference",
      message: "private calendar contents",
    })).toEqual({
      code: "classifier_inference_timeout",
      boundary: "auto_route_classifier_inference",
      kind: "timeout",
    });
    expect(autoRouteRecoveryEvidence({
      code: "/Users/private/secret",
      boundary: "Calendar / private",
    })).toEqual({
      code: "auto_route_unavailable",
      boundary: null,
      kind: "unknown",
    });
  });
});
