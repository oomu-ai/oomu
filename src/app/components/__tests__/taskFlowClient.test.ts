import { describe, expect, it } from "vitest";
import {
  taskFlowExecutionIsVerified,
  type TaskFlowExecutionResponse,
} from "../taskFlowClient";

function response(
  overrides: Partial<TaskFlowExecutionResponse> = {},
): TaskFlowExecutionResponse {
  return {
    completed_steps: 1,
    halted: false,
    flow: {
      flow_id: "flow-1",
      mission_id: "mission-1",
      parent_session_id: "session-1",
      directive: "Write a verified report.",
      status: "verified",
      steps: [
        {
          step_id: "step-1",
          sequence: 1,
          status: "verified",
          pre_conditions: [],
          action: {},
          post_conditions: [],
        },
      ],
      decision_nodes: [],
      heartbeats: [],
      created_at_ms: 1,
      updated_at_ms: 2,
    },
    ...overrides,
  };
}

describe("taskFlowExecutionIsVerified", () => {
  it("accepts only a fully verified native flow and all of its steps", () => {
    expect(taskFlowExecutionIsVerified(response())).toBe(true);
    expect(taskFlowExecutionIsVerified(response({ completed_steps: 0 }))).toBe(false);
    expect(
      taskFlowExecutionIsVerified(
        response({ flow: { ...response().flow, status: "active" } }),
      ),
    ).toBe(false);
    expect(
      taskFlowExecutionIsVerified(
        response({
          flow: {
            ...response().flow,
            steps: [{ ...response().flow.steps[0], status: "active" }],
          },
        }),
      ),
    ).toBe(false);
  });
});
