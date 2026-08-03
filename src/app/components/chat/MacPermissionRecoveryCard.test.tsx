import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import {
  macPermissionRecoveryDescriptor,
  MacPermissionRecoveryCard,
} from "./MacPermissionRecoveryCard";

afterEach(cleanup);

describe("Apple permission recovery card", () => {
  it("distinguishes supported permission states without guessing from unrelated failures", () => {
    expect(macPermissionRecoveryDescriptor("contacts_permission_denied", "contacts"))
      .toEqual({ capabilityId: "contacts", state: "denied" });
    expect(macPermissionRecoveryDescriptor("photos_permission_limited", "photos"))
      .toEqual({ capabilityId: "photos", state: "limited" });
    expect(macPermissionRecoveryDescriptor("camera_permission_restricted", "camera"))
      .toEqual({ capabilityId: "camera", state: "restricted" });
    expect(macPermissionRecoveryDescriptor("photos_authorization_revoked", "photos"))
      .toEqual({ capabilityId: "photos", state: "denied" });
    expect(macPermissionRecoveryDescriptor("tool_failed", "calendar")).toBeNull();
    expect(macPermissionRecoveryDescriptor("permission_denied", "made_up_app")).toBeNull();
  });

  it("opens the exact settings pane, checks once, and preserves a cancel path", async () => {
    const onOpenSettings = vi.fn(async () => undefined);
    const onCheck = vi.fn(async () => undefined);
    const onCancel = vi.fn(async () => undefined);
    render(
      <MacPermissionRecoveryCard
        boundary="contacts_authorization"
        code="contacts_permission_denied"
        descriptor={{ capabilityId: "contacts", state: "denied" }}
        recoveryId="execution-301"
        onCancel={onCancel}
        onCheck={onCheck}
        onOpenSettings={onOpenSettings}
        t={(key, variables) => {
          const values: Record<string, string> = {
            "sprint_299.permissions.capabilities.contacts.name": "Contacts",
            "sprint_301.permission_recovery.title": "{capability} access needed",
            "sprint_301.permission_recovery.denied_body": "Allow {capability} in System Settings to continue.",
            "sprint_301.permission_recovery.saved_recoverable": "Your request is saved.",
            "sprint_301.permission_recovery.open_settings": "Open System Settings",
            "sprint_301.permission_recovery.opening": "Opening…",
            "sprint_301.permission_recovery.check_again": "Check again",
            "sprint_301.permission_recovery.checking": "Checking…",
            "sprint_301.permission_recovery.cancel": "Cancel request",
            "sprint_301.auto_route_recovery.technical_details": "Show technical details",
            "sprint_301.auto_route_recovery.error_code": "Error code",
            "sprint_301.auto_route_recovery.stopped_at": "Stopped at",
          };
          let value = values[key] ?? key;
          Object.entries(variables ?? {}).forEach(([name, replacement]) => {
            value = value.replace(`{${name}}`, String(replacement));
          });
          return value;
        }}
      />,
      { wrapper: I18nProvider },
    );
    expect(screen.getByRole("alert")).toHaveFocus();
    fireEvent.click(screen.getByRole("button", { name: "Open System Settings" }));
    await waitFor(() => expect(onOpenSettings).toHaveBeenCalledWith("execution-301", "contacts"));
    fireEvent.click(screen.getByRole("button", { name: "Check again" }));
    await waitFor(() => expect(onCheck).toHaveBeenCalledWith("execution-301", "contacts"));
    fireEvent.click(screen.getByRole("button", { name: "Cancel request" }));
    await waitFor(() => expect(onCancel).toHaveBeenCalledWith("execution-301"));
  });

  it("does not offer a looping Settings action for a restricted capability", () => {
    render(
      <MacPermissionRecoveryCard
        boundary="camera_authorization"
        code="camera_permission_restricted"
        descriptor={{ capabilityId: "camera", state: "restricted" }}
        recoveryId="execution-302"
        onCancel={vi.fn(async () => undefined)}
        onCheck={vi.fn(async () => undefined)}
        onOpenSettings={vi.fn(async () => undefined)}
        t={(key) => key}
      />,
    );
    expect(screen.queryByRole("button", { name: /open_settings/ })).toBeNull();
    expect(screen.queryByRole("button", { name: /check_again/ })).toBeNull();
    expect(screen.getByRole("button", { name: /cancel/ })).toBeVisible();
  });

  it("retries a timeout without sending the user to System Settings", () => {
    render(
      <MacPermissionRecoveryCard
        boundary="calendar_authorization"
        code="calendar_permission_timeout"
        descriptor={{ capabilityId: "calendar", state: "timeout" }}
        recoveryId="execution-303"
        onCancel={vi.fn(async () => undefined)}
        onCheck={vi.fn(async () => undefined)}
        onOpenSettings={vi.fn(async () => undefined)}
        t={(key) => key}
      />,
    );
    expect(screen.queryByRole("button", { name: /open_settings/ })).toBeNull();
    expect(screen.getByRole("button", { name: /check_again/ })).toBeVisible();
    expect(screen.getByText("sprint_301.permission_recovery.saved_recoverable")).toBeVisible();
  });
});
