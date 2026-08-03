import { beforeEach, describe, expect, it, vi } from "vitest";
import { taskApi } from "./taskClient";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@/lib/invoke", () => ({ invoke: mocks.invoke }));

const taskIdentity = {
  runtimeRecordId: "workflow-instance-1",
  taskId: "task_11111111-1111-4111-8111-111111111111",
  taskRunId: "taskrun_22222222-2222-4222-8222-222222222222",
};

describe("task effect verification client", () => {
  beforeEach(() => mocks.invoke.mockReset().mockResolvedValue({}));

  it("stops an uninspectable effect without inventing action details", async () => {
    await taskApi.resolveEffectVerification({
      ...taskIdentity,
      decision: "stop_without_repeating",
    });

    expect(mocks.invoke).toHaveBeenCalledWith(
      "resolve_task_effect_verification",
      {
        request: {
          ...taskIdentity,
          decision: "stop_without_repeating",
        },
      },
    );
  });

  it("keeps the exact effect identity when stopping from loaded details", async () => {
    await taskApi.resolveEffectVerification({
      ...taskIdentity,
      decision: "stop_without_repeating",
      effectKind: "create_system_calendar_event",
      idempotencyKey: "effect-calendar-1",
      nodeId: "calendar-node",
      verificationSequence: 7,
    });

    expect(mocks.invoke).toHaveBeenCalledWith(
      "resolve_task_effect_verification",
      {
        request: expect.objectContaining({
          decision: "stop_without_repeating",
          idempotencyKey: "effect-calendar-1",
          verificationSequence: 7,
        }),
      },
    );
  });
});
