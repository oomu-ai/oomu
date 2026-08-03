import { beforeEach, describe, expect, it } from "vitest";
import { consumeTaskFocus, peekTaskFocus, requestTaskFocus } from "./taskFocus";

describe("task focus handoff", () => {
  beforeEach(() => window.sessionStorage.clear());

  it("hands a validated Task and its view state to the Task center exactly once", () => {
    requestTaskFocus("task_run-123", "completed");
    expect(peekTaskFocus()).toEqual({
      state: "completed",
      taskRunId: "task_run-123",
    });
    expect(consumeTaskFocus()).toEqual({
      state: "completed",
      taskRunId: "task_run-123",
    });
    expect(consumeTaskFocus()).toBeNull();
  });

  it("rejects malformed ids", () => {
    requestTaskFocus("../../private/task");
    expect(consumeTaskFocus()).toBeNull();
  });

  it("opens legacy id-only handoffs in the all-results fallback", () => {
    window.sessionStorage.setItem("oomu.tasks.focus", "task_run-legacy");
    expect(consumeTaskFocus()).toEqual({
      state: null,
      taskRunId: "task_run-legacy",
    });
  });
});
