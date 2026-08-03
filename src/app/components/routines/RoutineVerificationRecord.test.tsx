import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  buildRoutineVerificationRecord,
  RoutineVerificationRecord,
  type RoutineVerificationHistoryItem,
} from "./RoutineVerificationRecord";
import type { RoutineRecord } from "./routineClient";

vi.mock("@/context/I18nContext", () => ({
  useI18n: () => ({
    t: (key: string) => ({
      "common.copied": "Copied",
      "routines.copy_verification_record": "Copy verification record",
      "routines.verification_record_copied": "Verification record copied.",
      "routines.verification_record_copy_failed": "OOMU couldn’t copy the verification record. Try again.",
    })[key] ?? key,
  }),
}));

const routine: RoutineRecord = {
  routineId: "routine_task_11111111-1111-4111-8111-111111111111",
  label: "Morning brief",
  projectId: "project_11111111-1111-4111-8111-111111111111",
  workflowId: "workflow-1",
  workflowVersion: 4,
  scheduleExpression: "0 9 * * *",
  scheduleKind: "recurring",
  timezone: "America/New_York",
  isActive: true,
  nextRunAtMs: 2_000,
  nextRunsMs: [2_000],
  missedRunPolicy: "run_once",
  consecutiveFailures: 0,
  failureThreshold: 3,
  deliveryTarget: { destination: "private@example.com" },
};

const historyItem: RoutineVerificationHistoryItem = {
  taskRunId: "taskrun_22222222-2222-4222-8222-222222222222",
  taskId: "task_22222222-2222-4222-8222-222222222222",
  runtimeRecordId: "workflow-instance-2",
  executionInstanceId: "workflow-instance-2",
  correlationId: "correlation-2",
  state: "running",
  summary: "PRIVATE RESULT BODY",
  lastError: "RAW PROVIDER RECEIPT",
  createdAtMs: 1_100,
  updatedAtMs: 1_200,
  scheduledForMs: 900,
  runCreatedAtMs: 1_000,
  scheduleCreatedAtMs: 100,
  scheduleUpdatedAtMs: 200,
  scheduleNextRunAtMs: 2_000,
  effects: [{
    idempotencyKey: "calendar-effect-1",
    effectKind: "system_calendar_event",
    state: "verified",
    resultDigest: "effect-digest",
    updatedAtMs: 1_150,
  }],
  deliveryReceipts: [{
    receiptId: "delivery-task-2",
    platform: "slack",
    eventKind: "completed",
    state: "delivered",
    providerReceiptHash: "provider-message-hash",
    createdAtMs: 1_160,
    updatedAtMs: 1_170,
  }],
};

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("RoutineVerificationRecord", () => {
  it("copies exact pending and completed identity without private content", () => {
    const pending = buildRoutineVerificationRecord(routine, historyItem);
    expect(pending.run.state).toBe("running");
    expect(pending.run.runtimeRecordId).toBe("workflow-instance-2");
    expect(pending.effects[0]).toMatchObject({
      idempotencyKey: "calendar-effect-1",
      state: "verified",
    });
    expect(pending.deliveryReceipts[0]).toMatchObject({
      receiptId: "delivery-task-2",
      providerMessageIdHash: "provider-message-hash",
    });

    const completed = buildRoutineVerificationRecord(routine, {
      ...historyItem,
      state: "completed",
    });
    expect(completed.run.state).toBe("completed");

    const encoded = JSON.stringify(completed);
    expect(encoded).not.toContain("PRIVATE RESULT BODY");
    expect(encoded).not.toContain("RAW PROVIDER RECEIPT");
    expect(encoded).not.toContain("private@example.com");
    expect(encoded).not.toContain("deliveryTarget");
  });

  it("confirms a successful copy visibly and to assistive technology", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    render(<RoutineVerificationRecord item={historyItem} routine={routine} />);

    fireEvent.click(screen.getByRole("button", { name: "Copy verification record" }));

    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(1));
    expect(JSON.parse(writeText.mock.calls[0][0])).toEqual(
      buildRoutineVerificationRecord(routine, historyItem),
    );
    expect(screen.getByRole("button", { name: "Copied" })).toBeVisible();
    expect(screen.getByRole("status")).toHaveTextContent("Verification record copied.");
  });

  it("keeps a failed copy actionable and explains what happened", async () => {
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: vi.fn().mockRejectedValue(new Error("clipboard unavailable")) },
    });
    render(<RoutineVerificationRecord item={historyItem} routine={routine} />);

    fireEvent.click(screen.getByRole("button", { name: "Copy verification record" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "OOMU couldn’t copy the verification record. Try again.",
    );
    expect(screen.getByRole("button", { name: "Copy verification record" })).toBeEnabled();
  });
});
