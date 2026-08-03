import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { RoutineHandoffNotice } from "./RoutineHandoffNotice";

afterEach(cleanup);

describe("RoutineHandoffNotice", () => {
  it("keeps the exact request visible and describes only confirmed future behavior", () => {
    const translations: Record<string, string> = {
      "routines.handoff_request_title": "Requested in Chat",
      "routines.handoff_schedule_seeded": "Review every field.",
      "routines.handoff_timing_defaulted": "Review the default starting point.",
      "routines.handoff_cadence_boundary_conflict": "No future weekly run fits before midnight.",
      "routines.handoff_run_once_pending": "The test run is queued only after confirmation.",
      "routines.handoff_midnight_enforced": "Future runs stop at midnight after confirmation.",
    };
    const request =
      "Check my unread email every week until midnight. Once you set it up, run it once.";
    render(
      <RoutineHandoffNotice
        draft={{
          id: "draft-1",
          requestText: request,
          scheduleText: "every week",
          scheduleKind: "recurring",
          cadence: { interval: 1, unit: "week" },
          scheduleSupported: true,
          timingDefaulted: true,
          cadenceBoundaryConflict: true,
          runOnceRequested: true,
          endBoundary: "midnight",
        }}
        t={(key) => translations[key] ?? key}
      />,
    );

    expect(screen.getByText(request)).toBeVisible();
    expect(screen.getByText("Review the default starting point.")).toBeVisible();
    expect(screen.getByText("No future weekly run fits before midnight.")).toBeVisible();
    expect(screen.getByText("The test run is queued only after confirmation.")).toBeVisible();
    expect(screen.getByText("Future runs stop at midnight after confirmation.")).toBeVisible();
  });

  it("asks for correction instead of claiming an unsupported cadence", () => {
    render(
      <RoutineHandoffNotice
        draft={{
          id: "draft-2",
          requestText: "Check unread email every 30 seconds.",
          scheduleText: "every 30 seconds",
          scheduleKind: "recurring",
          cadence: null,
          scheduleSupported: false,
          timingDefaulted: false,
          cadenceBoundaryConflict: false,
          runOnceRequested: false,
          endBoundary: null,
        }}
        t={(key) => key === "routines.handoff_schedule_unsupported"
          ? "Choose an interval of at least one minute."
          : key}
      />,
    );

    expect(screen.getByText("Choose an interval of at least one minute.")).toBeVisible();
  });
});
