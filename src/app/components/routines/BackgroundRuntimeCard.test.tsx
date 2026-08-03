import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { BackgroundRuntimeCard } from "./BackgroundRuntimeCard";
import type { BackgroundStatus } from "./routineClient";

const copy: Record<string, string> = {
  "common.loading": "Loading…",
  "routines.background": "Work in the background",
  "routines.background_state_off": "Off",
  "routines.background_state_turning_on": "Turning on…",
  "routines.background_state_on": "On",
  "routines.background_state_needs_attention": "Needs attention",
  "routines.background_state_turning_off": "Turning off…",
  "routines.background_checking_help": "OOMU is checking background work.",
  "routines.background_check_failed": "OOMU couldn’t check background work. Try again.",
  "routines.background_off_help": "Scheduled tasks run only while OOMU is open.",
  "routines.background_turning_on_help": "OOMU is checking that background work can run.",
  "routines.background_on_help": "OOMU can keep scheduled tasks moving after you close the window.",
  "routines.background_needs_attention_help": "Background work stopped. Try again, or turn it off.",
  "routines.background_turning_off_help": "OOMU is stopping background work.",
  "routines.background_turn_on": "Turn on",
  "routines.background_turn_off": "Turn off",
  "routines.background_try_again": "Try again",
  "routines.background_repair_label": "Background work options",
  "routines.background_approval_help": "Allow OOMU in System Settings, General, Login Items. Then check again.",
  "routines.background_open_login_items": "Open Login Items",
  "routines.background_recheck": "Check again",
  "routines.background_signed_install_help": "Use a signed installed copy of OOMU for background work.",
  "routines.updating": "Updating…",
};

const t = (key: string) => copy[key] ?? key;

function status(
  state: BackgroundStatus["state"],
  userEnabled = state !== "off",
  errorCode: string | null = state === "needs_attention" ? "background_runtime_worker_stopped" : null,
): BackgroundStatus {
  return {
    userEnabled,
    verifiedActive: state === "on_verified",
    state,
    registrationState: state === "off" ? "unregistered" : "registered",
    registrationBackend: "supervised_process",
    processState: state === "on_verified" ? "running" : "absent",
    registrationGeneration: state === "off" ? null : "registration-1",
    processId: state === "on_verified" ? 42 : null,
    buildNumber: 7,
    buildIdentity: "build-7",
    profileClass: "development",
    profileGenerationSha256: "profile-digest-1",
    heartbeatAtMs: state === "on_verified" ? 1 : null,
    heartbeatAgeMs: state === "on_verified" ? 0 : null,
    menuVisible: state === "on_verified",
    errorCode,
    detail: "BACKEND CANARY. Never render this.",
    checkedAtMs: 1,
    recentReceipts: [],
  };
}

afterEach(cleanup);

describe("BackgroundRuntimeCard state", () => {

  it("shows On only for a verified runtime and offers one obvious off action", () => {
    const onChange = vi.fn();
    const { container } = render(
      <BackgroundRuntimeCard
        busy={false}
        disabled={false}
        error=""
        onChange={onChange}
        onOpenLoginItems={vi.fn()}
        onRefresh={vi.fn()}
        status={status("on_verified")}
        t={t}
      />,
    );

    expect(container.querySelector("#background-runtime-control")).not.toBeNull();
    expect(container.querySelector("#background-runtime-state")).toHaveTextContent("On");
    fireEvent.click(screen.getByRole("button", { name: "Turn off" }));
    expect(onChange).toHaveBeenCalledWith(false);
    expect(screen.queryByText(/BACKEND CANARY|private_backend_code/)).toBeNull();
  });

  it("gives a calm inline recovery choice when verified background work stops", () => {
    const onChange = vi.fn();
    const { container } = render(
      <BackgroundRuntimeCard
        busy={false}
        disabled={false}
        error=""
        onChange={onChange}
        onOpenLoginItems={vi.fn()}
        onRefresh={vi.fn()}
        status={status("needs_attention")}
        t={t}
      />,
    );

    expect(container.querySelector("#background-runtime-repair")).toHaveAccessibleName(
      "Background work options",
    );
    expect(screen.getByText("Background work stopped. Try again, or turn it off.")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    fireEvent.click(screen.getByRole("button", { name: "Turn off" }));
    expect(onChange).toHaveBeenNthCalledWith(1, true);
    expect(onChange).toHaveBeenNthCalledWith(2, false);
  });

  it("prevents duplicate requests while a native transition is in progress", () => {
    render(
      <BackgroundRuntimeCard
        busy={false}
        disabled={false}
        error=""
        onChange={vi.fn()}
        onOpenLoginItems={vi.fn()}
        onRefresh={vi.fn()}
        status={status("turning_on")}
        t={t}
      />,
    );

    expect(screen.getByRole("button", { name: "Updating…" })).toBeDisabled();
  });
});

describe("BackgroundRuntimeCard recovery", () => {
  it("opens Login Items and rechecks only after macOS approval can change", () => {
    const onChange = vi.fn();
    const onOpenLoginItems = vi.fn();
    render(
      <BackgroundRuntimeCard
        busy={false}
        disabled={false}
        error=""
        onChange={onChange}
        onOpenLoginItems={onOpenLoginItems}
        onRefresh={vi.fn()}
        status={status("needs_attention", true, "background_requires_approval")}
        t={t}
      />,
    );

    expect(screen.getByText(/System Settings, General, Login Items/)).toBeVisible();
    expect(screen.queryByRole("button", { name: "Try again" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Open Login Items" }));
    fireEvent.click(screen.getByRole("button", { name: "Check again" }));
    expect(onOpenLoginItems).toHaveBeenCalledOnce();
    expect(onChange).toHaveBeenCalledWith(true);
  });

  it("does not offer a dead retry when a signed installed copy is required", () => {
    render(
      <BackgroundRuntimeCard
        busy={false}
        disabled={false}
        error=""
        onChange={vi.fn()}
        onOpenLoginItems={vi.fn()}
        onRefresh={vi.fn()}
        status={status("needs_attention", true, "background_requires_signed_install")}
        t={t}
      />,
    );

    expect(screen.getByText("Use a signed installed copy of OOMU for background work."))
      .toBeVisible();
    expect(screen.queryByRole("button", { name: "Try again" })).toBeNull();
    expect(screen.getByRole("button", { name: "Turn off" })).toBeVisible();
  });

  it("turns a failed status check into one clear, accessible retry", () => {
    const onRefresh = vi.fn();
    const onChange = vi.fn();
    render(
      <BackgroundRuntimeCard
        busy={false}
        disabled={false}
        error="OOMU couldn’t check background work. Try again."
        onChange={onChange}
        onOpenLoginItems={vi.fn()}
        onRefresh={onRefresh}
        status={null}
        t={t}
      />,
    );

    expect(screen.getByText("Needs attention")).toBeVisible();
    expect(screen.getAllByText("OOMU couldn’t check background work. Try again.")).toHaveLength(1);
    expect(screen.getByRole("group", { name: "Background work options" })).toHaveAccessibleDescription(
      "OOMU couldn’t check background work. Try again.",
    );
    expect(screen.getByRole("button", { name: "Turn off" })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    fireEvent.click(screen.getByRole("button", { name: "Turn off" }));
    expect(onRefresh).toHaveBeenCalledOnce();
    expect(onChange).toHaveBeenCalledWith(false);
  });
});
