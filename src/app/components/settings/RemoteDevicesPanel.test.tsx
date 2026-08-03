import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { RemoteDevicesPanel } from "./RemoteDevicesPanel";

const invokeMock = vi.hoisted(() => vi.fn());

const labels: Record<string, string> = {
  "common.cancel": "Cancel",
  "common.loading": "Loading…",
  "common.refresh": "Refresh",
  "common.refreshing": "Refreshing",
  "remote_devices.abilities.check": "Can check tasks",
  "remote_devices.connected": "Devices you trust",
  "remote_devices.empty": "No remote devices are connected.",
  "remote_devices.errors.load": "OOMU couldn't load your remote devices.",
  "remote_devices.errors.remove": "OOMU couldn't remove this device.",
  "remote_devices.not_used": "Not used yet",
  "remote_devices.refreshed": "The trusted-device list is up to date.",
  "remote_devices.remove": "Remove",
  "remote_devices.remove_action": "Remove device",
  "remote_devices.remove_help": "This device will no longer control OOMU.",
  "remote_devices.remove_title": "Remove Alex's phone?",
  "remote_devices.removing": "Removing…",
  "remote_devices.subtitle": "Review devices that have been trusted.",
  "remote_devices.title": "Remote devices",
  "remote_devices.unavailable_help": "OOMU needs a companion phone app and secure connection before a phone can start or check work.",
  "remote_devices.unavailable_title": "Phone access isn’t available yet",
};

function translate(key: string, variables?: Record<string, string | number>) {
  if (key === "remote_devices.removed") {
    return `${variables?.name} was removed.`;
  }
  return labels[key] ?? key;
}

vi.mock("@/context/I18nContext", () => ({
  useI18n: () => ({
    t: translate,
  }),
}));

vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));

const phone = {
  allowedProjectIds: ["project-1"],
  expiresAtMs: Date.now() + 100_000,
  label: "Alex's phone",
  lastUsedAtMs: null,
  pairedAtMs: Date.now(),
  remoteDeviceId: "device-1",
  revokedAtMs: null,
  scopes: ["view_task"],
};

describe("RemoteDevicesPanel", () => {
  beforeEach(() => {
    invokeMock.mockReset().mockResolvedValue([]);
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("does not show an unusable QR code or pairing action", async () => {
    render(<RemoteDevicesPanel />);

    expect(
      await screen.findByText("Phone access isn’t available yet"),
    ).toBeVisible();
    expect(screen.queryByRole("img")).toBeNull();
    expect(screen.queryByRole("button", { name: /pair|code|confirm/i })).toBeNull();
    expect(invokeMock).not.toHaveBeenCalledWith(
      "create_remote_pairing_challenge",
      expect.anything(),
    );
  });

  it("shows working and success feedback while refreshing real device state", async () => {
    render(<RemoteDevicesPanel />);
    await screen.findByText("No remote devices are connected.");

    let resolveRefresh: (devices: unknown[]) => void = () => undefined;
    invokeMock.mockImplementationOnce(
      () => new Promise<unknown[]>((resolve) => { resolveRefresh = resolve; }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));

    const refreshing = screen.getByRole("button", { name: "Refreshing" });
    expect(refreshing).toHaveAttribute("aria-busy", "true");
    expect(refreshing).toHaveAttribute("data-action-state", "working");

    resolveRefresh([]);
    expect(await screen.findByRole("status")).toHaveTextContent(
      "The trusted-device list is up to date.",
    );
  });

  it("removes a trusted device with visible progress and confirmation", async () => {
    invokeMock.mockResolvedValueOnce([phone]);
    render(<RemoteDevicesPanel />);
    await screen.findByText("Alex's phone");

    fireEvent.click(screen.getByRole("button", { name: "Remove" }));
    const dialog = screen.getByRole("dialog", { name: "Remove Alex's phone?" });
    expect(dialog).toHaveAttribute("aria-modal", "true");
    expect(screen.getByRole("button", { name: "Cancel" })).toHaveFocus();

    let resolveRemoval: (value: unknown) => void = () => undefined;
    invokeMock.mockImplementationOnce(
      () => new Promise((resolve) => { resolveRemoval = resolve; }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Remove device" }));

    expect(screen.getByRole("button", { name: "Removing…" })).toHaveAttribute(
      "data-action-state",
      "working",
    );
    resolveRemoval(phone);

    await waitFor(() => expect(screen.queryByText("Alex's phone")).toBeNull());
    expect(screen.getByRole("status")).toHaveTextContent("Alex's phone was removed.");
    expect(invokeMock).toHaveBeenCalledWith("revoke_remote_device", {
      request: { remoteDeviceId: "device-1" },
    });
    expect(screen.getByRole("button", { name: "Refresh" })).toHaveAttribute(
      "data-action-state",
      "idle",
    );
  });

  it("closes the removal dialog with Escape and returns focus", async () => {
    invokeMock.mockResolvedValueOnce([phone]);
    render(<RemoteDevicesPanel />);
    await screen.findByText("Alex's phone");

    const removeButton = screen.getByRole("button", { name: "Remove" });
    fireEvent.click(removeButton);
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    await waitFor(() => expect(removeButton).toHaveFocus());
    expect(invokeMock).not.toHaveBeenCalledWith("revoke_remote_device", expect.anything());
  });
});
