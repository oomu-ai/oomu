import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { RoutineDeliveryStatus } from "./RoutineDeliveryStatus";
import { RoutineHistoryOutcome } from "./RoutineHistoryOutcome";
import { routineApi } from "./routineClient";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@/lib/invoke", () => ({ invoke: mocks.invoke }));

const copy: Record<string, string> = {
  "routines.delivery_retrying_title": "Finishing delivery",
  "routines.delivery_retrying_body":
    "Your work is safe. OOMU will retry the private-channel update without rerunning the task.",
  "routines.delivery_review_title": "Check the private channel",
  "routines.delivery_review_body":
    "Your work is safe, but OOMU couldn't confirm whether the update arrived.",
  "routines.delivery_review_action": "I checked — send it again",
  "routines.delivery_retrying_action": "Preparing delivery…",
  "routines.history_completed_with_declined_actions":
    "Finished with your choices",
  "routines.history_declined_actions": "Not performed: {actions}",
  "tasks.state_failed": "Failed",
};
const t = (key: string, values?: Record<string, string | number>) => {
  let value = copy[key] ?? key;
  Object.entries(values ?? {}).forEach(([name, replacement]) => {
    value = value.replaceAll(`{${name}}`, String(replacement));
  });
  return value;
};

describe("Routine delivery recovery", () => {
  beforeEach(() => mocks.invoke.mockReset());

  it("keeps a proven pre-dispatch retry calm and automatic", () => {
    render(
      <RoutineDeliveryStatus
        busy={false}
        disabled={false}
        onRetry={vi.fn()}
        state="retrying"
        t={t}
      />,
    );
    expect(screen.getByText("Finishing delivery")).toBeVisible();
    expect(screen.getByText(/Your work is safe/)).toBeVisible();
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("asks for an absence check before an uncertain delivery can be retried", () => {
    const onRetry = vi.fn();
    render(
      <RoutineDeliveryStatus
        busy={false}
        disabled={false}
        onRetry={onRetry}
        state="needs_review"
        t={t}
      />,
    );
    fireEvent.click(
      screen.getByRole("button", { name: "I checked — send it again" }),
    );
    expect(onRetry).toHaveBeenCalledOnce();
  });

  it("sends the explicit absence confirmation through the command boundary", async () => {
    mocks.invoke.mockResolvedValue({});
    await routineApi.retryDelivery("routine-1");
    expect(mocks.invoke).toHaveBeenCalledWith("retry_routine_delivery", {
      request: { routineId: "routine-1", confirmedAbsent: true },
    });
  });

  it("names declined actions as a completed choice rather than a failure", () => {
    render(
      <RoutineHistoryOutcome
        item={{
          state: "failed",
          summary: "Finished",
          createdAtMs: 1,
          updatedAtMs: 2,
          outcome: "completed_with_declined_actions",
          declinedActions: ["Send the email"],
        }}
        t={t}
      />,
    );
    expect(screen.getByText("Finished with your choices")).toBeVisible();
    expect(screen.getByText("Not performed: Send the email")).toBeVisible();
    expect(screen.queryByText("Failed")).toBeNull();
  });
});
