import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import {
  actionKey,
  AppControlMonitor,
  outcomeKey,
  pauseReasonKey,
} from "./AppControlMonitor";
import type {
  AppControlActionKind,
  AppControlPauseReason,
  AppControlSessionView,
} from "./appControlClient";

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));

const TASK_RUN_ID = "taskrun_55555555-5555-4555-8555-555555555555";

function session(
  overrides: Partial<AppControlSessionView> = {},
): AppControlSessionView {
  return {
    sessionId: "desktop_11111111-1111-4111-8111-111111111111",
    taskRunId: TASK_RUN_ID,
    projectId: "project_22222222-2222-4222-8222-222222222222",
    state: "running",
    application: { name: "Numbers", icon: "numbers" },
    currentAction: {
      kind: "type_text",
      targetLabel: "RAW TARGET CANARY",
      willChangeData: true,
    },
    pauseReason: null,
    canPause: true,
    canTakeControl: true,
    canReturnToOomu: false,
    observationGeneration: 77,
    lastOutcome: {
      status: "verified",
      actionKind: "type_text",
      receiptId: "RAW RECEIPT CANARY",
      recordedAtMs: Date.parse("2026-07-12T00:00:00Z"),
      detailsAvailable: true,
    },
    updatedAtMs: Date.parse("2026-07-12T00:00:00Z"),
    ...overrides,
  };
}

function localeState() {
  return {
    activeLocale: "en-US",
    availableLocales: [{
      id: "en-US",
      label: "English (US)",
      fileName: "en-US.json",
      isDefault: true,
      verified: true,
    }],
    translations: {},
  };
}

function renderMonitor(taskRunId: string | null = TASK_RUN_ID) {
  return render(
    <AppControlMonitor pollIntervalMs={0} taskRunId={taskRunId ?? undefined} />,
    { wrapper: I18nProvider },
  );
}

describe("AppControlMonitor", () => {
  let current: AppControlSessionView | null;

  beforeEach(() => {
    current = session();
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (command: string, payload?: unknown) => {
      if (command === "get_locale_state") return localeState();
      if (command === "get_app_control_status") return current;
      if (command === "control_app_control_session") {
        const control = (payload as { request: { control: string } }).request.control;
        if (control === "take_control") {
          current = session({
            state: "takeover",
            currentAction: null,
            canPause: false,
            canTakeControl: false,
            canReturnToOomu: true,
          });
        } else if (control === "return_to_oomu") {
          current = session({
            state: "return_pending",
            currentAction: null,
            canReturnToOomu: false,
          });
        } else if (control === "pause") {
          current = session({
            state: "paused",
            currentAction: null,
            pauseReason: "secure_field",
            canPause: false,
            canReturnToOomu: true,
          });
        }
        return current;
      }
      return null;
    });
  });

  afterEach(cleanup);

  it("keeps Take control one click away beside one human activity line without raw fields", async () => {
    renderMonitor();

    expect(await screen.findByText("OOMU is working in Numbers.")).toBeVisible();
    expect(screen.getByText("Entering information in Numbers.")).toBeVisible();
    expect(screen.getByRole("button", { name: "Pause" })).toBeEnabled();
    const takeover = screen.getByRole("button", { name: "Take control" });
    expect(takeover).toBeEnabled();
    expect(takeover.className).toContain("inverse-background");
    expect(screen.queryByRole("combobox")).toBeNull();
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(screen.getByText("Last checked")).not.toBeVisible();

    const rendered = document.body.textContent ?? "";
    for (const hidden of [
      "RAW TARGET CANARY",
      "RAW RECEIPT CANARY",
      "type_text",
      "project_22222222",
      "desktop_11111111",
      "element reference",
      "postcondition",
      "bundle id",
      "accessibility tree",
      "semantic snapshot",
    ]) {
      expect(rendered.toLowerCase()).not.toContain(hidden.toLowerCase());
    }
  });

  it("hands control to the user with calm copy and one obvious return action", async () => {
    renderMonitor();
    fireEvent.click(await screen.findByRole("button", { name: "Take control" }));

    expect(await screen.findByText("You're driving in Numbers.")).toBeVisible();
    expect(screen.getByText("OOMU paused and is watching. Hand it back whenever you're ready.")).toBeVisible();
    expect(screen.getByRole("button", { name: "Return to OOMU" })).toBeEnabled();
    expect(invokeMock).toHaveBeenCalledWith(
      "control_app_control_session",
      { request: { sessionId: session().sessionId, taskRunId: TASK_RUN_ID, control: "take_control" } },
    );
  });

  it("pauses immediately without disabling the user's control", async () => {
    renderMonitor();
    fireEvent.click(await screen.findByRole("button", { name: "Pause" }));

    expect(await screen.findByText("OOMU stopped at a password field. It won't type here—you take it from here.")).toBeVisible();
    expect(screen.getByRole("button", { name: "Continue" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Take control" })).toBeEnabled();
    expect(invokeMock).toHaveBeenCalledWith(
      "control_app_control_session",
      { request: { sessionId: session().sessionId, taskRunId: TASK_RUN_ID, control: "pause" } },
    );
  });

  it("rechecks the app after handback and never implies a queued replay", async () => {
    current = session({
      state: "takeover",
      currentAction: null,
      canPause: false,
      canTakeControl: false,
      canReturnToOomu: true,
    });
    let finishHandback: ((value: AppControlSessionView) => void) | undefined;
    const handback = new Promise<AppControlSessionView>((resolve) => { finishHandback = resolve; });
    invokeMock.mockImplementation(async (command: string, payload?: unknown) => {
      if (command === "get_locale_state") return localeState();
      if (command === "get_app_control_status") return current;
      if (command === "control_app_control_session") {
        expect((payload as { request: { control: string } }).request.control).toBe("return_to_oomu");
        return handback;
      }
      return null;
    });
    renderMonitor();
    fireEvent.click(await screen.findByRole("button", { name: "Return to OOMU" }));

    expect(await screen.findByText("OOMU is getting ready to continue in Numbers.")).toBeVisible();
    expect(screen.getByText("OOMU is looking at the screen again before it continues.")).toBeVisible();
    expect(screen.getByRole("button", { name: "Pause" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Take control" })).toBeEnabled();
    expect(document.body.textContent?.toLowerCase()).not.toContain("replay");
    expect(invokeMock).toHaveBeenCalledWith(
      "control_app_control_session",
      { request: { sessionId: session().sessionId, taskRunId: TASK_RUN_ID, control: "return_to_oomu" } },
    );
    current = session({ state: "running" });
    finishHandback?.(current);
    expect(await screen.findByText("OOMU is working in Numbers.")).toBeVisible();
  });

  it("maps a safety pause to direct, blame-free guidance", async () => {
    current = session({
      state: "paused",
      currentAction: null,
      pauseReason: "secure_field",
      canPause: false,
      canReturnToOomu: true,
    });
    renderMonitor();

    expect(await screen.findByText("OOMU stopped at a password field. It won't type here—you take it from here.")).toBeVisible();
    expect(screen.getByRole("button", { name: "Continue" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Take control" })).toBeEnabled();
  });

  it("shows a mapped outcome summary and keeps supporting detail collapsed", async () => {
    current = session({ state: "completed", currentAction: null });
    renderMonitor();

    expect(await screen.findByText("OOMU finished in Numbers.")).toBeVisible();
    expect(screen.getAllByText("OOMU checked that this step finished as expected.")[0]).toBeVisible();
    expect(screen.queryByRole("button", { name: "Take control" })).toBeNull();
    expect(screen.getByText("Last checked")).not.toBeVisible();
    fireEvent.click(screen.getByText("Details"));
    expect(screen.getByText("Last checked")).toBeVisible();
    expect(screen.getAllByText("OOMU checked that this step finished as expected.")).toHaveLength(2);
  });

  it("uses safe fallback labels for every unknown backend value", async () => {
    current = session({
      state: "RAW_STATE_CANARY" as AppControlSessionView["state"],
      application: { name: "Preview", icon: "RAW_ICON_CANARY" as AppControlSessionView["application"] extends { icon: infer T } ? T : never },
      currentAction: { kind: "RAW_ACTION_CANARY" as AppControlActionKind, targetLabel: "RAW TARGET CANARY", willChangeData: true },
      pauseReason: "RAW_REASON_CANARY" as AppControlPauseReason,
      lastOutcome: { ...session().lastOutcome!, status: "RAW_OUTCOME_CANARY" as AppControlSessionView["lastOutcome"] extends { status: infer T } ? T : never },
    });
    renderMonitor();

    expect(await screen.findByText("OOMU is paused in Preview.")).toBeVisible();
    expect(screen.getByText("OOMU paused before continuing. Take control if you want to review the app.")).toBeVisible();
    const rendered = document.body.textContent ?? "";
    for (const canary of ["RAW_STATE_CANARY", "RAW_ICON_CANARY", "RAW_ACTION_CANARY", "RAW_REASON_CANARY", "RAW_OUTCOME_CANARY", "RAW TARGET CANARY"]) {
      expect(rendered).not.toContain(canary);
    }
  });

  it("never shows an identifier-shaped app name", async () => {
    current = session({ application: { name: "com.example.internal-app", icon: "generic" } });
    renderMonitor();

    expect(await screen.findByText("OOMU is working in an app.")).toBeVisible();
    expect(document.body.textContent).not.toContain("com.example.internal-app");
  });

  it("supports the global AppShell hook without a Task identifier", async () => {
    renderMonitor(null);
    await screen.findByText("OOMU is working in Numbers.");
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "get_app_control_status",
      { request: { taskRunId: null } },
    ));
  });

  it("keeps both safety controls available when a control request fails", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "get_app_control_status") return current;
      if (command === "control_app_control_session") throw new Error("offline");
      return null;
    });
    renderMonitor();
    fireEvent.click(await screen.findByRole("button", { name: "Take control" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "OOMU could not change control right now. You can take over directly in the app.",
    );
    expect(screen.getByRole("button", { name: "Pause" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Take control" })).toBeEnabled();
  });
});

describe("app-control enum label boundaries", () => {
  it.each([
    ["focus", "app_control.activity.focus"],
    ["press", "app_control.activity.press"],
    ["select", "app_control.activity.select"],
    ["type_text", "app_control.activity.type"],
    ["invoke_menu", "app_control.activity.invoke_menu"],
    ["scroll", "app_control.activity.scroll"],
    ["drag_drop", "app_control.activity.drag_drop"],
    ["choose_file", "app_control.activity.choose_file"],
    ["apple_event", "app_control_actions.activate"],
  ] as const)("maps action %s", (value, key) => {
    expect(actionKey(value)).toBe(key);
  });

  it.each([
    ["user_input", "app_control.pause_reason.user_input"],
    ["secure_field", "app_control.pause_reason.secure_field"],
    ["ambiguous_target", "app_control.pause_reason.ambiguous_target"],
    ["repeated_mismatch", "app_control.pause_reason.repeated_mismatch"],
    ["unexpected_navigation", "app_control.pause_reason.unexpected_navigation"],
    ["permission_changed", "app_control.pause_reason.permission_changed"],
    ["hidden_window", "app_control.pause_reason.hidden_window"],
    ["application_changed", "app_control.pause_reason.application_changed"],
    ["driver_unavailable", "app_control.pause_reason.control_unavailable"],
  ] as const)("maps pause reason %s", (value, key) => {
    expect(pauseReasonKey(value)).toBe(key);
  });

  it("maps result states and unknown values without exposing raw codes", () => {
    expect(outcomeKey("verified", "completed")).toBe("app_control.outcome.verified");
    expect(outcomeKey("verified", "failed")).toBe("app_control.outcome.failed");
    expect(outcomeKey("verified", "stopped")).toBe("app_control.outcome.stopped");
    expect(outcomeKey("raw" as never, "completed")).toBe("app_control.outcome.completed");
    expect(actionKey("raw" as never)).toBe("app_control.activity.unknown");
    expect(pauseReasonKey("raw" as never)).toBe("app_control.pause_reason.unknown");
  });
});
