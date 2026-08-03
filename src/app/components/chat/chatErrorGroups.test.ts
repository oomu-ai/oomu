import { describe, expect, it } from "vitest";

import { chatErrorGroup } from "./chatErrorGroups";

describe("chatErrorGroup native approval recovery", () => {
  it("renders a denied plan approval as an intentional permission decision", () => {
    expect(chatErrorGroup("authority_user_denied")).toBe("permission_denied");
  });

  it.each([
    "authority_native_prompt_window_unavailable",
    "authority_native_prompt_timeout",
    "shield_native_prompt_window_unavailable",
  ])("renders %s with the localized retryable permission copy", (code) => {
    expect(chatErrorGroup(code)).toBe("permission_request");
  });

  it.each([
    "planner_connector_binding_mismatch",
    "connector_planned_account_not_found",
    "connector_planned_manifest_mismatch",
    "connector_planned_account_mismatch",
    "connector_planned_adapter_unavailable",
    "connector_planned_account_reconnect_required",
    "connector_planned_project_context_required",
    "connector_planned_project_invalid",
    "connector_planned_project_authorization_required",
    "connector_planned_capability_unsupported",
    "connector_planned_capability_consent_required",
  ])("groups %s as a connector authority problem", (code) => {
    expect(chatErrorGroup(code)).toBe("connector_authority");
  });
});
