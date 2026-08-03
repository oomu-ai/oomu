import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { RoutineHistoryOutcome } from "./RoutineHistoryOutcome";
import type { RoutineHistoryItem } from "./routineClient";

const labels: Record<string, string> = {
  "routines.history_failure_details": "Technical details",
  "routines.history_failure_generic":
    "OOMU stopped before finishing. Open the result to review it or retry from the safe checkpoint.",
  "routines.history_failure_official_source":
    "OOMU couldn’t read any approved official source and stopped before later steps. Nothing after that source was changed. Open the result to retry.",
  "routines.history_failure_title": "This run needs your attention",
  "routines.history_state_failed": "Failed",
};

const t = (key: string) => labels[key] ?? key;

afterEach(cleanup);

describe("RoutineHistoryOutcome", () => {
  it("explains an official-source failure and keeps technical detail secondary", () => {
    const item: RoutineHistoryItem = {
      state: "failed",
      summary: "Scheduled evidence report",
      lastErrorCode: "official_page_fetch_failed",
      lastError: "The official page returned HTTP 403.",
      createdAtMs: 1,
      updatedAtMs: 2,
    };

    render(<RoutineHistoryOutcome item={item} t={t} />);

    expect(screen.getByText("This run needs your attention")).toBeVisible();
    expect(
      screen.getByText(labels["routines.history_failure_official_source"]),
    ).toBeVisible();
    expect(screen.getByText("Technical details")).toBeVisible();
    expect(
      screen.getByText("The official page returned HTTP 403."),
    ).toBeInTheDocument();
  });
});
