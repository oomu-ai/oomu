import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TrustPanel } from "./TrustPanel";

const invokeMock = vi.hoisted(() => vi.fn());

const labels: Record<string, string> = {
  "common.browse": "Browse",
  "common.browsing": "Browsing…",
  "common.refresh": "Refresh",
  "common.refreshing": "Refreshing",
  "common.save": "Save",
  "settings.privacy.trust.allowed_tools": "Allowed tools",
  "settings.privacy.trust.audit_rows_count": "0 audit rows",
  "settings.privacy.trust.browse_error": "OOMU couldn't open the folder browser.",
  "settings.privacy.trust.choose_folder_title": "Choose a folder to trust",
  "settings.privacy.trust.execution_audit": "Execution audit",
  "settings.privacy.trust.folder_path": "Folder path",
  "settings.privacy.trust.folders_count": "0 folders",
  "settings.privacy.trust.no_active_sessions": "No active sessions",
  "settings.privacy.trust.no_audit_rows": "No audit rows",
  "settings.privacy.trust.no_trusted_folders": "No trusted folders",
  "settings.privacy.trust.reviewed_scopes": "Reviewed scopes",
  "settings.privacy.trust.reviewed_scopes_empty": "No reviewed scopes",
  "settings.privacy.trust.sessions_count": "0 sessions",
  "settings.privacy.trust.tier_global": "Global",
  "settings.privacy.trust.tier_session": "Session",
  "settings.privacy.trust.title": "Trust Dashboard",
  "settings.privacy.trust.tool_shell": "Commands",
  "settings.privacy.trust.tool_writes": "File changes",
  "settings.privacy.trust.trust_tier": "Trust tier",
  "settings.privacy.trust.trusted_folders": "Trusted folders",
};

vi.mock("@/context/I18nContext", () => ({
  useI18n: () => ({
    t: (key: string) => labels[key] ?? key,
  }),
}));

vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));

describe("TrustPanel path selection", () => {
  beforeEach(() => {
    invokeMock.mockReset().mockImplementation((command: string) => {
      if (command === "get_sovereign_trust_dashboard") {
        return Promise.resolve({ activeSessions: [], auditEvents: [], policies: [] });
      }
      if (command === "get_reviewed_approval_scopes") {
        return Promise.resolve({ auditEvents: [], grants: [] });
      }
      return Promise.reject(new Error(`Unexpected command: ${command}`));
    });
  });

  afterEach(cleanup);

  it("uses the native folder picker and keeps the selected path visible", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_sovereign_trust_dashboard") {
        return Promise.resolve({ activeSessions: [], auditEvents: [], policies: [] });
      }
      if (command === "get_reviewed_approval_scopes") {
        return Promise.resolve({ auditEvents: [], grants: [] });
      }
      if (command === "choose_directory_path") {
        return Promise.resolve("/Users/example/Projects/Important");
      }
      return Promise.reject(new Error(`Unexpected command: ${command}`));
    });

    render(<TrustPanel />);
    fireEvent.click(await screen.findByRole("button", { name: "Browse" }));

    await waitFor(() => {
      expect(screen.getByLabelText("Folder path")).toHaveValue(
        "/Users/example/Projects/Important",
      );
    });
    expect(screen.getByRole("button", { name: "Browse" })).toHaveAttribute(
      "data-action-state",
      "success",
    );
    expect(screen.getByRole("button", { name: "Save" })).toHaveAttribute(
      "data-action-state",
      "idle",
    );
    expect(invokeMock).toHaveBeenCalledWith("choose_directory_path", {
      initialPath: "~/Projects/OOMU",
      title: "Choose a folder to trust",
    });
  });

  it("shows a bounded localized error when the native picker cannot open", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_sovereign_trust_dashboard") {
        return Promise.resolve({ activeSessions: [], auditEvents: [], policies: [] });
      }
      if (command === "get_reviewed_approval_scopes") {
        return Promise.resolve({ auditEvents: [], grants: [] });
      }
      if (command === "choose_directory_path") {
        return Promise.reject(new Error("NATIVE PATH CANARY"));
      }
      return Promise.reject(new Error(`Unexpected command: ${command}`));
    });

    render(<TrustPanel />);
    fireEvent.click(await screen.findByRole("button", { name: "Browse" }));

    expect(await screen.findByText("OOMU couldn't open the folder browser.")).toBeVisible();
    expect(screen.queryByText("NATIVE PATH CANARY")).toBeNull();
    expect(screen.getByRole("button", { name: "Browse" })).toHaveAttribute(
      "data-action-state",
      "error",
    );
    expect(screen.getByRole("button", { name: "Save" })).toHaveAttribute(
      "data-action-state",
      "idle",
    );
  });
});
