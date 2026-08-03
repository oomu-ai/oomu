import { describe, expect, it } from "vitest";
import type { P0EventEnvelope } from "@/lib/p0Contracts";
import type { TaskRun } from "./taskClient";
import { taskEffectVerificationFromEvents } from "./taskEffectVerification";

const task = {
  effectVerificationRequired: true,
  runtimeRecordId: "instance-1",
  taskId: "task_11111111-1111-4111-8111-111111111111",
  taskRunId: "taskrun_22222222-2222-4222-8222-222222222222",
} as TaskRun;

function event(
  sequence: number,
  eventType: string,
  payload: Record<string, unknown>,
): P0EventEnvelope {
  return {
    correlationId: "correlation-1",
    evidenceClass: "observed_result",
    eventType,
    payload,
    projectId: "project_33333333-3333-4333-8333-333333333333",
    schemaVersion: 1,
    sequence,
    taskId: task.taskId,
    taskRunId: task.taskRunId,
    timestamp: "2026-07-21T12:00:00.000Z",
  } as P0EventEnvelope;
}

describe("protected action verification projection", () => {
  it("projects only the bounded Calendar identity needed for recovery", () => {
    const projected = taskEffectVerificationFromEvents(task, [
      event(4, "workflow.effect.verification_required", {
        effectKind: "create_system_calendar_event",
        effectSummary: {
          calendarName: "OOMU Test",
          notes: "private notes must not be projected",
          surface: "calendar",
          title: "Supplier Decision Review",
        },
        idempotencyKey: "effect-1",
        nodeId: "calendar-node",
        retrySupported: true,
      }),
    ]);

    expect(projected).toEqual({
      calendarName: "OOMU Test",
      effectKind: "create_system_calendar_event",
      idempotencyKey: "effect-1",
      nodeId: "calendar-node",
      retrySupported: true,
      subject: undefined,
      recipient: undefined,
      surface: "calendar",
      title: "Supplier Decision Review",
      verificationSequence: 4,
    });
    expect(JSON.stringify(projected)).not.toContain("private notes");
  });

  it("removes a boundary after its exact audited resolution", () => {
    const required = event(4, "workflow.effect.verification_required", {
      effectKind: "send_system_email",
      effectSummary: { surface: "mail_send", recipient: "person@example.com", subject: "Brief" },
      idempotencyKey: "effect-1",
      nodeId: "mail-node",
      retrySupported: true,
    });
    const resolved = event(5, "workflow.effect.verification_resolved", {
      effectKind: "send_system_email",
      idempotencyKey: "effect-1",
      nodeId: "mail-node",
      verificationSequence: 4,
    });

    expect(taskEffectVerificationFromEvents(task, [required, resolved])).toBeNull();
  });
});
