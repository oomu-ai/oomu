import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { RoutineIdentityDetails } from "./RoutineIdentityDetails";
import type { RoutineRecord } from "./routineClient";

vi.mock("@/context/I18nContext", () => ({
  useI18n: () => ({
    t: (key: string) =>
      ({
        "routines.identity_details": "Schedule details",
        "routines.routine_id": "Routine ID",
        "routines.workflow_version": "Workflow version",
      })[key] ?? key,
  }),
}));

const routine: RoutineRecord = {
  routineId: "routine-ship-05",
  label: "Ship Test 05 — Unattended Brief",
  workflowId: "workflow-operations-brief",
  workflowVersion: 7,
  scheduleExpression: "on 2026-07-22 at 09:30",
  scheduleKind: "one_shot",
  timezone: "America/New_York",
  isActive: true,
  nextRunsMs: [],
  missedRunPolicy: "skip",
  consecutiveFailures: 0,
  failureThreshold: 3,
  deliveryTarget: {},
};

describe("RoutineIdentityDetails", () => {
  it("exposes the durable routine identity before the first run", () => {
    render(<RoutineIdentityDetails routine={routine} />);

    expect(screen.getByText("Schedule details")).toBeInTheDocument();
    expect(screen.getByText("Routine ID")).toBeInTheDocument();
    expect(screen.getByText("routine-ship-05")).toBeInTheDocument();
    expect(screen.getByText("Workflow version")).toBeInTheDocument();
    expect(screen.getByText("7")).toBeInTheDocument();
  });
});
