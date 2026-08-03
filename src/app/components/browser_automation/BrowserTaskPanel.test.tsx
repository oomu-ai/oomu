import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import enUS from "@/locales/en-US.json";
import {
  browserActivityKey,
  browserHeadlineKey,
  browserStatusKey,
  BrowserTaskPanel,
} from "./BrowserTaskPanel";
import type {
  BrowserActionResult,
  BrowserAutomationState,
  BrowserSession,
  BrowserSnapshot,
} from "./browserClient";

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));

const TASK_RUN_ID = "taskrun_55555555-5555-4555-8555-555555555555";
const PROJECT_ID = "project_22222222-2222-4222-8222-222222222222";

function snapshot(overrides: Partial<BrowserSnapshot> = {}): BrowserSnapshot {
  return {
    documentGeneration: 91,
    url: "https://raw-origin-canary.example/private",
    title: "Quarterly report",
    capturedAtMs: Date.parse("2026-07-12T00:00:00Z"),
    nodes: [{
      role: "RAW ROLE CANARY",
      name: "RAW NODE NAME CANARY",
      valueClass: "RAW VALUE CANARY",
      visible: true,
      enabled: true,
      reference: "RAW REFERENCE CANARY",
    }],
    possiblePromptInjection: false,
    protectedInterruption: null,
    ...overrides,
  };
}

function browserSession(overrides: Partial<BrowserSession> = {}): BrowserSession {
  return {
    sessionId: "browser_RAW_SESSION_CANARY",
    taskRunId: TASK_RUN_ID,
    projectId: PROJECT_ID,
    canonicalOrigin: "https://raw-origin-canary.example",
    destinationBinding: "RAW DESTINATION CANARY",
    state: "automating",
    documentGeneration: 0,
    currentStep: "RAW CURRENT STEP CANARY",
    lastSnapshotAtMs: null,
    snapshot: null,
    ...overrides,
  };
}

function actionResult(observation: BrowserSnapshot | null): BrowserActionResult {
  return {
    state: "RAW RESULT STATE CANARY",
    observation,
    screenshotPath: "/RAW/SCREENSHOT/CANARY.png",
    message: "RAW RESULT MESSAGE CANARY",
  };
}

function t(key: string, values: Record<string, string | number> = {}) {
  let current: unknown = enUS;
  for (const segment of key.split(".")) {
    if (!current || typeof current !== "object" || Array.isArray(current)) return key;
    current = (current as Record<string, unknown>)[segment];
  }
  if (typeof current !== "string") return key;
  return Object.entries(values).reduce(
    (copy, [name, value]) => copy.split(`{${name}}`).join(String(value)),
    current,
  );
}

function renderPanel() {
  return render(
    <BrowserTaskPanel
      pollIntervalMs={0}
      projectId={PROJECT_ID}
      t={t}
      taskRunId={TASK_RUN_ID}
    />,
  );
}

describe("BrowserTaskPanel", () => {
  let current: BrowserSession;
  let nextSnapshot: BrowserSnapshot;

  beforeEach(() => {
    current = browserSession();
    nextSnapshot = snapshot();
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (command: string, payload?: unknown) => {
      if (command === "start_browser_automation") return current;
      if (command === "get_browser_automation_session") return current;
      if (command === "execute_browser_action") {
        current = {
          ...current,
          state: "automating",
          documentGeneration: nextSnapshot.documentGeneration,
          lastSnapshotAtMs: nextSnapshot.capturedAtMs,
          snapshot: nextSnapshot,
        };
        return actionResult(nextSnapshot);
      }
      if (command === "control_browser_automation") {
        const control = (payload as { request: { control: string } }).request.control;
        current = {
          ...current,
          state: control === "pause"
            ? "paused"
            : control === "takeover"
              ? "takeover"
              : control === "return"
                ? "return_pending"
                : "stopped",
        };
        return current;
      }
      return null;
    });
  });

  afterEach(cleanup);

  async function startPanel() {
    renderPanel();
    const start = screen.getByRole("button", { name: "Let OOMU use this page" });
    expect(start).toBeDisabled();
    fireEvent.click(screen.getByRole("checkbox"));
    expect(start).toBeEnabled();
    fireEvent.click(start);
    await screen.findByText("OOMU is using the browser.");
  }

  it("starts with one clear choice, checks the page automatically, and keeps Take control one click away", async () => {
    await startPanel();

    expect(screen.getByText("Working on Quarterly report.")).toBeVisible();
    expect(screen.getByRole("button", { name: "Pause" })).toBeEnabled();
    const takeover = screen.getByRole("button", { name: "Take control" });
    expect(takeover).toBeEnabled();
    expect(takeover.className).toContain("inverse-background");
    expect(invokeMock).toHaveBeenCalledWith("start_browser_automation", {
      request: {
        taskRunId: TASK_RUN_ID,
        projectId: PROJECT_ID,
        projectPolicyConsent: true,
      },
    });
    expect(invokeMock).toHaveBeenCalledWith("execute_browser_action", {
      request: {
        sessionId: current.sessionId,
        taskRunId: TASK_RUN_ID,
        projectId: PROJECT_ID,
        action: { kind: "snapshot" },
        step: "Check the open page before continuing",
        expectedPostcondition: null,
      },
    });
    expect(screen.queryByRole("combobox")).toBeNull();
    expect(screen.queryByRole("textbox")).toBeNull();
    for (const removed of ["Retry safely", "Fresh snapshot", "Reload", "Stop"]) {
      expect(screen.queryByRole("button", { name: removed })).toBeNull();
    }
  });

  it("never renders raw page-control fields or engineer-console vocabulary", async () => {
    await startPanel();
    const rendered = document.body.textContent ?? "";
    for (const hidden of [
      "RAW ROLE CANARY",
      "RAW NODE NAME CANARY",
      "RAW VALUE CANARY",
      "RAW REFERENCE CANARY",
      "RAW CURRENT STEP CANARY",
      "RAW DESTINATION CANARY",
      "RAW RESULT STATE CANARY",
      "RAW RESULT MESSAGE CANARY",
      "raw-origin-canary.example",
      "document generation",
      "element reference",
      "postcondition",
      "semantic snapshot",
    ]) {
      expect(rendered.toLowerCase()).not.toContain(hidden.toLowerCase());
    }
  });

  it("hands the browser to the user with calm copy and one obvious return action", async () => {
    await startPanel();
    fireEvent.click(screen.getByRole("button", { name: "Take control" }));

    expect(await screen.findByText("You're driving now.")).toBeVisible();
    expect(screen.getByText("OOMU paused and is watching. Hand it back whenever you're ready.")).toBeVisible();
    expect(screen.getByRole("button", { name: "Return to OOMU" })).toBeEnabled();
    expect(invokeMock).toHaveBeenCalledWith("control_browser_automation", {
      request: { sessionId: current.sessionId, taskRunId: TASK_RUN_ID, control: "takeover" },
    });
  });

  it("checks the page again after handback before continuing", async () => {
    await startPanel();
    fireEvent.click(screen.getByRole("button", { name: "Take control" }));
    await screen.findByRole("button", { name: "Return to OOMU" });

    let resolveCheck!: (value: BrowserActionResult) => void;
    const check = new Promise<BrowserActionResult>((resolve) => { resolveCheck = resolve; });
    invokeMock.mockImplementation(async (command: string, payload?: unknown) => {
      if (command === "control_browser_automation") {
        current = { ...current, state: "return_pending" };
        return current;
      }
      if (command === "execute_browser_action") return check;
      if (command === "get_browser_automation_session") return current;
      const control = (payload as { request?: { control?: string } } | undefined)?.request?.control;
      return control ? current : null;
    });

    fireEvent.click(screen.getByRole("button", { name: "Return to OOMU" }));
    expect(await screen.findByText("OOMU is getting ready to continue.")).toBeVisible();
    expect(screen.getByText("OOMU is checking the page again before it continues.")).toBeVisible();
    expect(screen.getByRole("button", { name: "Take control" })).toBeEnabled();

    const refreshed = snapshot({ title: "Updated report", documentGeneration: 92 });
    await act(async () => { resolveCheck(actionResult(refreshed)); await check; });
    expect(await screen.findByText("Working on Updated report.")).toBeVisible();
    expect(invokeMock).toHaveBeenCalledWith("control_browser_automation", {
      request: { sessionId: current.sessionId, taskRunId: TASK_RUN_ID, control: "return" },
    });
  });

  it("honors takeover requested during the required handback check", async () => {
    await startPanel();
    fireEvent.click(screen.getByRole("button", { name: "Take control" }));
    await screen.findByRole("button", { name: "Return to OOMU" });

    let resolveCheck!: (value: BrowserActionResult) => void;
    const check = new Promise<BrowserActionResult>((resolve) => { resolveCheck = resolve; });
    invokeMock.mockImplementation(async (command: string, payload?: unknown) => {
      if (command === "control_browser_automation") {
        const control = (payload as { request: { control: string } }).request.control;
        current = { ...current, state: control === "return" ? "return_pending" : "takeover" };
        return current;
      }
      if (command === "execute_browser_action") return check;
      if (command === "get_browser_automation_session") return current;
      return null;
    });

    fireEvent.click(screen.getByRole("button", { name: "Return to OOMU" }));
    fireEvent.click(await screen.findByRole("button", { name: "Take control" }));
    expect(screen.getByText("OOMU is checking the page again before it continues.")).toBeVisible();

    await act(async () => { resolveCheck(actionResult(snapshot())); await check; });
    expect(await screen.findByText("You're driving now.")).toBeVisible();
    const controls = invokeMock.mock.calls
      .filter(([command]) => command === "control_browser_automation")
      .map(([, payload]) => (payload as { request: { control: string } }).request.control);
    expect(controls.slice(-2)).toEqual(["return", "takeover"]);
  });

  it("pauses without exposing controls that the backend cannot honor", async () => {
    await startPanel();
    fireEvent.click(screen.getByRole("button", { name: "Pause" }));

    expect(await screen.findByText("OOMU paused in the browser.")).toBeVisible();
    expect(screen.getByText("OOMU stopped before taking another step.")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Pause" })).toBeNull();
    expect(screen.getByRole("button", { name: "Take control" })).toBeEnabled();
  });

  it("turns page hazards into calm, non-technical takeover guidance", async () => {
    nextSnapshot = snapshot({
      possiblePromptInjection: true,
      protectedInterruption: "RAW PROTECTED KIND CANARY",
    });
    await startPanel();

    expect(screen.getByText("This page may be trying to give OOMU instructions. Take control to review it.")).toBeVisible();
    expect(screen.getByText("This page needs you to finish a protected step. Take control when you're ready.")).toBeVisible();
    expect(document.body.textContent).not.toContain("RAW PROTECTED KIND CANARY");
  });

  it("keeps mapped supporting information collapsed", async () => {
    await startPanel();
    expect(screen.getByText("Last checked")).not.toBeVisible();
    fireEvent.click(screen.getByText("Details"));
    expect(screen.getByText("Last checked")).toBeVisible();
    expect(screen.getByText("Status")).toBeVisible();
    expect(screen.getByText("Working")).toBeVisible();
    expect(screen.queryByText("automating")).toBeNull();
  });

  it("shows safe fallback copy for unknown states and backend failures", async () => {
    current = browserSession({ state: "RAW STATE CANARY" as BrowserAutomationState });
    renderPanel();
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(screen.getByRole("button", { name: "Let OOMU use this page" }));
    expect(await screen.findByText("OOMU paused in the browser.")).toBeVisible();
    expect(document.body.textContent).not.toContain("RAW STATE CANARY");

    cleanup();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "start_browser_automation") throw new Error("RAW ERROR CANARY");
      return null;
    });
    renderPanel();
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(screen.getByRole("button", { name: "Let OOMU use this page" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "OOMU could not start using this browser. Nothing changed.",
    );
    expect(document.body.textContent).not.toContain("RAW ERROR CANARY");
  });

  it("keeps the supervisor controls visible after a control request fails", async () => {
    await startPanel();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "control_browser_automation") throw new Error("offline");
      if (command === "get_browser_automation_session") return current;
      return null;
    });
    fireEvent.click(screen.getByRole("button", { name: "Take control" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "OOMU could not change control right now. You can take over directly in the browser.",
    );
    expect(screen.getByRole("button", { name: "Pause" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Take control" })).toBeEnabled();
  });
});

describe("browser supervisor label boundaries", () => {
  it.each([
    ["automating", "browser.headline.automating", "browser.status.automating"],
    ["paused", "browser.headline.paused", "browser.status.paused"],
    ["takeover", "browser.headline.takeover", "browser.status.takeover"],
    ["return_pending", "browser.headline.return_pending", "browser.status.return_pending"],
    ["stopped", "browser.headline.stopped", "browser.status.stopped"],
    ["closed", "browser.headline.closed", "browser.status.closed"],
  ] as const)("maps %s without rendering the state value", (state, headline, status) => {
    expect(browserHeadlineKey(state)).toBe(headline);
    expect(browserStatusKey(state)).toBe(status);
  });

  it("maps activity and unknown values to closed copy", () => {
    expect(browserActivityKey("automating", false)).toBe("browser.activity.checking");
    expect(browserActivityKey("automating", true)).toBe("browser.activity.working");
    expect(browserActivityKey("raw" as never, true)).toBe("browser.activity.paused");
    expect(browserHeadlineKey("raw" as never)).toBe("browser.headline.paused");
  });
});
