import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { invoke } from "@/lib/invoke";
import { MacPermissionsPanel, permissionAction, type MacPermissionStatus } from "./MacPermissionsPanel";

vi.mock("@/lib/invoke", () => ({ invoke: vi.fn() }));

const states: MacPermissionStatus[] = [
  { capabilityId: "calendar", state: "allowed", canRequest: false, settingsPane: "Privacy_Calendars", checkedAtMs: 1 },
  { capabilityId: "screen_capture", state: "denied", canRequest: false, settingsPane: "Privacy_ScreenCapture", checkedAtMs: 1 },
  { capabilityId: "microphone", state: "when_used", canRequest: false, settingsPane: "Privacy_Microphone", checkedAtMs: 1 },
  { capabilityId: "camera", state: "not_requested", canRequest: true, settingsPane: "Privacy_Camera", checkedAtMs: 1 },
  { capabilityId: "files_and_folders", state: "when_used", canRequest: false, settingsPane: "Privacy_FilesAndFolders", checkedAtMs: 1 },
  { capabilityId: "full_disk_access", state: "allowed", canRequest: false, settingsPane: "Privacy_AllFiles", checkedAtMs: 1 },
  { capabilityId: "local_network", state: "when_used", canRequest: false, settingsPane: "Privacy_LocalNetwork", checkedAtMs: 1 },
  { capabilityId: "music", state: "allowed", canRequest: false, settingsPane: "Privacy_Media", checkedAtMs: 1 },
  { capabilityId: "notifications", state: "unsupported", canRequest: false, settingsPane: "Notifications", checkedAtMs: 1 },
];

function localeState() {
  return { activeLocale: "en-US", availableLocales: [{ id: "en-US", label: "English (US)", fileName: "en-US.json", isDefault: true, verified: true }], translations: {} };
}

describe("Mac permission settings", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_macos_permission_states") return states;
      if (command === "request_macos_permission") return { ...states[3], state: "allowed", canRequest: false };
      return null;
    });
  });
  afterEach(cleanup);

  it("shows live typed states and exactly one useful action for each unresolved permission", async () => {
    render(<MacPermissionsPanel />, { wrapper: I18nProvider });
    expect(await screen.findByText("Calendar")).toBeVisible();
    expect(screen.getByText("Screen recording")).toBeVisible();
    expect(screen.getByText("Files and folders")).toBeVisible();
    expect(screen.getByText("Full Disk Access")).toBeVisible();
    expect(screen.getByText("Local network")).toBeVisible();
    expect(screen.getByText("Music")).toBeVisible();
    expect(screen.getByText("Lets OOMU see what is on your screen when you ask.")).toBeVisible();
    expect(screen.getByRole("button", { name: "Open System Settings" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Allow" })).toBeVisible();
    expect(screen.queryByRole("button", { name: /Calendar/ })).toBeNull();
  });

  it("requests a first-use permission through the native broker", async () => {
    render(<MacPermissionsPanel />, { wrapper: I18nProvider });
    fireEvent.click(await screen.findByRole("button", { name: "Allow" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("request_macos_permission", {
      request: { capabilityId: "camera" },
    }));
  });

  it("shows unbundled Notifications as unavailable without offering an action", async () => {
    render(<MacPermissionsPanel />, { wrapper: I18nProvider });
    const heading = await screen.findByRole("heading", { name: "Notifications" });
    const row = heading.closest(".flex.flex-col");
    expect(row).not.toBeNull();
    expect(within(row as HTMLElement).getByText("Not available")).toBeVisible();
    expect(within(row as HTMLElement).getByText(
      "Lets OOMU tell you when work finishes or needs attention.",
    )).toBeVisible();
    expect(within(row as HTMLElement).queryByRole("button")).toBeNull();
  });

  it("waits for the user to return from System Settings before checking halted work", async () => {
    const dispatch = vi.spyOn(window, "dispatchEvent");
    render(<MacPermissionsPanel />, { wrapper: I18nProvider });
    fireEvent.click(await screen.findByRole("button", { name: "Open System Settings" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("open_macos_permission_settings", {
      request: { capabilityId: "screen_capture" },
    }));
    expect(dispatch.mock.calls.some(([event]) => event.type === "oomu:macos-permissions-refreshed")).toBe(false);
    fireEvent(window, new Event("focus"));
    await waitFor(() => expect(dispatch.mock.calls.some(([event]) =>
      event.type === "oomu:macos-permissions-refreshed",
    )).toBe(true));
    dispatch.mockRestore();
  });

  it("never exposes a raw native error in the primary alert", async () => {
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_macos_permission_states") throw new Error("TCC kTCCServiceCalendar -1743");
      return null;
    });
    render(<MacPermissionsPanel />, { wrapper: I18nProvider });
    expect(await screen.findByRole("alert")).toHaveTextContent("OOMU couldn’t check Mac permissions.");
    expect(screen.queryByText(/-1743|kTCCService/)).toBeNull();
  });

  it("derives actions from typed state rather than internal error text", () => {
    expect(permissionAction(states[0])).toBeNull();
    expect(permissionAction(states[1])).toBe("settings");
    expect(permissionAction(states[2])).toBeNull();
    expect(permissionAction(states[3])).toBe("request");
  });
});
