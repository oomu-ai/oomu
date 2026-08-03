import { describe, expect, it } from "vitest";

import {
  formatTrustAction,
  formatTrustScopeKind,
  formatTrustStatus,
  isUntilRevokedReviewedScope,
} from "./TrustPanel";

const labels: Record<string, string> = {
  "settings.privacy.trust.scope_task": "This Task",
  "settings.privacy.trust.scope_project_path": "This Project folder",
  "settings.privacy.trust.scope_persistent": "Saved approval",
  "settings.privacy.trust.scope_other": "Other reviewed scope",
  "settings.privacy.trust.action_read_files": "Read files",
  "settings.privacy.trust.action_change_files": "Change files",
  "settings.privacy.trust.action_change_code": "Change code",
  "settings.privacy.trust.action_index_documents": "Add documents to search",
  "settings.privacy.trust.action_check_system": "Check this Mac",
  "settings.privacy.trust.action_control_app": "Control an app",
  "settings.privacy.trust.action_browser": "Use the browser",
  "settings.privacy.trust.action_other": "Other protected action",
  "settings.privacy.trust.status_approved": "Approved",
  "settings.privacy.trust.status_running": "In progress",
  "settings.privacy.trust.status_paused": "Paused",
  "settings.privacy.trust.status_unknown": "Status unavailable",
};

const t = (key: string) => labels[key] ?? `missing:${key}`;

describe("TrustPanel human labels", () => {
  it("maps stored approval scopes without exposing their identifiers", () => {
    expect(formatTrustScopeKind("task", t)).toBe("This Task");
    expect(formatTrustScopeKind("project_path", t)).toBe("This Project folder");
    expect(formatTrustScopeKind("persistent", t)).toBe("Saved approval");
    expect(formatTrustScopeKind("future_scope_kind", t)).toBe("Other reviewed scope");
    expect(formatTrustScopeKind("future_scope_kind", t)).not.toContain("future_scope_kind");
  });

  it.each([
    ["file_read", "Read files"],
    ["file_write", "Change files"],
    ["codebase_patch", "Change code"],
    ["document_index", "Add documents to search"],
    ["system_audit", "Check this Mac"],
    ["app_control", "Control an app"],
    ["browser_navigate", "Use the browser"],
  ])("maps the %s action to human copy", (value, expected) => {
    expect(formatTrustAction(value, t)).toBe(expected);
  });

  it("uses a safe action fallback instead of echoing an unknown identifier", () => {
    expect(formatTrustAction("future_action_class", t)).toBe("Other protected action");
    expect(formatTrustAction("future_action_class", t)).not.toContain("future_action_class");
  });

  it("maps execution states and safely handles new ones", () => {
    expect(formatTrustStatus("approved", t)).toBe("Approved");
    expect(formatTrustStatus("running", t)).toBe("In progress");
    expect(formatTrustStatus("actuation_lease_paused", t)).toBe("Paused");
    expect(formatTrustStatus("future_status_code", t)).toBe("Status unavailable");
    expect(formatTrustStatus("future_status_code", t)).not.toContain("future_status_code");
  });

  it("distinguishes an until-revoked approval from a finite persistent grant", () => {
    expect(isUntilRevokedReviewedScope({
      scopeKind: "persistent",
      expiresAtMs: Date.UTC(9999, 0, 1),
    })).toBe(true);
    expect(isUntilRevokedReviewedScope({
      scopeKind: "persistent",
      expiresAtMs: Date.now() + 86_400_000,
    })).toBe(false);
    expect(isUntilRevokedReviewedScope({
      scopeKind: "app_session",
      expiresAtMs: Date.UTC(9999, 0, 1),
    })).toBe(false);
    expect(isUntilRevokedReviewedScope({
      active: false,
      scopeKind: "persistent",
      expiresAtMs: Date.UTC(9999, 0, 1),
      revokedAtMs: Date.now(),
    })).toBe(false);
  });
});
