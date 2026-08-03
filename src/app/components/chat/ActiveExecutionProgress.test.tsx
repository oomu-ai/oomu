import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ActiveExecutionProgress } from "./ActiveExecutionProgress";
import type { ActiveAgentExecution } from "./agentExecutionState";

const developerBuild = vi.hoisted(() => ({ value: false }));

vi.mock("@/lib/buildFlags", () => ({
  get isDeveloperBuild() {
    return developerBuild.value;
  },
}));

const copy = {
  "chat.execution.title": "Working on your request · {status}",
  "chat.execution.step_many": "{count} steps",
  "chat.execution.step_one": "1 step",
  "chat.execution.technical_details": "Technical details",
  "chat.execution.waiting_logs": "Getting started…",
  "chat.execution.working": "Working…",
  "chat.execution.status.running": "Running",
  "common.track_in_tasks": "Track in Tasks",
  "trust.phases.authorize": "Checking permission",
  "trust.phases.step_running": "Doing the work",
} as const;

vi.mock("@/context/I18nContext", () => ({
  useI18n: () => ({
    t: (key: keyof typeof copy, variables?: Record<string, string | number>) => {
      let value: string = copy[key] ?? key;
      for (const [name, replacement] of Object.entries(variables ?? {})) {
        value = value.replaceAll(`{${name}}`, String(replacement));
      }
      return value;
    },
  }),
}));

const execution: ActiveAgentExecution = {
  executionId: "execution-raw-uuid-123",
  planId: "plan-raw-uuid-456",
  sessionId: "session-1",
  status: "running",
  logs: [
    {
      id: 1,
      executionId: "execution-raw-uuid-123",
      planId: "plan-raw-uuid-456",
      sessionId: "session-1",
      level: "info",
      phase: "authorize",
      message: "BACKEND RAW AUTHORIZATION LOG",
      createdAtMs: 1_750_000_000_000,
    },
    {
      id: 2,
      executionId: "execution-raw-uuid-123",
      planId: "plan-raw-uuid-456",
      sessionId: "session-1",
      level: "info",
      phase: "step_running",
      message: "BACKEND RAW EXECUTION LOG",
      createdAtMs: 1_750_000_001_000,
    },
  ],
  lastSeenId: 2,
  startedAtMs: 1_750_000_000_000,
};

describe("ActiveExecutionProgress", () => {
  afterEach(() => {
    developerBuild.value = false;
    cleanup();
  });

  it("shows an honest status card without diagnostics for ordinary users", () => {
    const view = render(<ActiveExecutionProgress execution={execution} />);

    expect(screen.getByText("Working on your request · Running")).toBeVisible();
    expect(screen.getByText("2 steps")).toBeVisible();
    expect(screen.getByRole("status")).toHaveTextContent("Working…");
    expect(screen.getByText("Checking permission")).toBeVisible();
    expect(screen.getByText("Doing the work")).toBeVisible();
    expect(screen.queryByText("execution-raw-uuid-123")).toBeNull();
    expect(screen.queryByText("plan-raw-uuid-456")).toBeNull();
    expect(screen.queryByText(/BACKEND RAW/)).toBeNull();
    expect(view.container.querySelector(".w-1\\/2")).toBeNull();
    expect(view.container.firstElementChild).not.toHaveAttribute(
      "data-agent-execution-id",
    );
  });

  it("keeps ids and raw logs inside developer-only technical details", () => {
    developerBuild.value = true;
    const view = render(<ActiveExecutionProgress execution={execution} />);

    fireEvent.click(screen.getByText("Technical details"));
    expect(view.container).toHaveTextContent("execution-raw-uuid-123");
    expect(view.container).toHaveTextContent("plan-raw-uuid-456");
    expect(view.container).toHaveTextContent("BACKEND RAW AUTHORIZATION LOG");
    expect(view.container.firstElementChild).toHaveAttribute(
      "data-agent-execution-id",
      "execution-raw-uuid-123",
    );
  });
});
