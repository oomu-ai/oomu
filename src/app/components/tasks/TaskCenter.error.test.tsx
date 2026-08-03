import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { TaskErrorSummary } from "./TaskCenter";

const labels: Record<string, string> = {
  "tasks.error_generic":
    "This task stopped before it finished. Review its activity, then retry it if that option is available.",
  "tasks.error_record_missing":
    "OOMU couldn't confirm this task finished, so it didn't mark it done.",
  "tasks.error_title": "What happened",
};

const t = (key: string) => labels[key] ?? key;

afterEach(cleanup);

describe("TaskErrorSummary", () => {
  it("renders a calm category without backend prose or machine identifiers", () => {
    const rawRecoveryCanary =
      "BACKEND CANARY: Owning runtime record is missing.";
    const rawUnknownCanary = "backend_failure_code_77";
    const view = render(
      <TaskErrorSummary
        lastError={rawRecoveryCanary}
        recoveryState="lost"
        t={t}
      />,
    );

    expect(
      screen.getByText(
        "OOMU couldn't confirm this task finished, so it didn't mark it done.",
      ),
    ).toBeVisible();
    expect(screen.queryByText(/BACKEND|Owning runtime/i)).toBeNull();

    view.rerender(
      <TaskErrorSummary
        lastError={rawUnknownCanary}
        recoveryState="reconciled"
        t={t}
      />,
    );

    expect(screen.getByText(labels["tasks.error_generic"])).toBeVisible();
    expect(screen.queryByText(rawUnknownCanary)).toBeNull();
  });
});
