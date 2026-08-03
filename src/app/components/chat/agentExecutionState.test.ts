import { describe, expect, it } from "vitest";
import {
  shouldSynthesizeResumeActionKey,
  statusFromExecutionLogs,
  type ActiveAgentExecution,
} from "./agentExecutionState";

function execution(status: ActiveAgentExecution["status"]): ActiveAgentExecution {
  return {
    executionId: "execution-1",
    planId: "plan-1",
    sessionId: "session-1",
    status,
    logs: [],
    lastSeenId: 7,
    startedAtMs: 1,
  };
}

describe("agent execution restart recovery state", () => {
  it("maps restart recovery readiness to a terminal halted state", () => {
    expect(statusFromExecutionLogs([{
      id: 8,
      executionId: "execution-1",
      planId: "plan-1",
      sessionId: "session-1",
      agentId: "agent-1",
      level: "info",
      phase: "restart_recovery_ready",
      message: "Restore the pending approval",
      payloadJson: null,
      createdAtMs: 2,
    }], "running")).toBe("halted");
  });

  it("lets a matching durable interruption override a stale running marker", () => {
    const staleRunning = execution("running");
    expect(shouldSynthesizeResumeActionKey(staleRunning, {
      executionId: "execution-1",
      planId: "plan-1",
    })).toBe(false);
    expect(shouldSynthesizeResumeActionKey(staleRunning, null)).toBe(true);
    expect(shouldSynthesizeResumeActionKey(staleRunning, {
      executionId: "another-execution",
      planId: "plan-1",
    })).toBe(true);
    expect(shouldSynthesizeResumeActionKey(execution("halted"), null)).toBe(false);
  });
});
