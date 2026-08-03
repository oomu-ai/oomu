import { act, cleanup, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@/lib/invoke";
import type { ActiveAgentExecution } from "./agentExecutionState";
import { useMacPermissionExecutionResume } from "./useMacPermissionExecutionResume";

vi.mock("@/lib/invoke", () => ({ invoke: vi.fn() }));

const execution: ActiveAgentExecution = {
  executionId: "execution-1",
  planId: "plan-1",
  sessionId: "session-1",
  status: "halted",
  logs: [],
  lastSeenId: 0,
  startedAtMs: 1,
};

const recoveryMessage = JSON.stringify({
  schema: "oomu.agent_execution_recovery.v1",
  executionId: "execution-1",
  planId: "plan-1",
  code: "calendar_permission_denied",
  boundary: "permission",
  recoverable: true,
  recoveryAction: "resume_same_execution",
  message: "Calendar permission is needed.",
  changedState: "checkpoint_saved",
  context: { capabilityId: "calendar" },
});

function Harness({ onResumed }: { onResumed: (executionId: string) => void }) {
  useMacPermissionExecutionResume({
    activeExecution: execution,
    activeSessionId: "session-1",
    messages: [{ content: recoveryMessage }],
    onResumed,
  });
  return null;
}

describe("macOS permission execution recovery", () => {
  beforeEach(() => vi.mocked(invoke).mockReset());
  afterEach(cleanup);

  it("resumes on mount when a permission event was missed", async () => {
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === "list_macos_permission_states") return [{ capabilityId: "calendar", state: "allowed" }];
      return { resumed: true, executionId: "execution-1", reason: "resumed" };
    });
    const onResumed = vi.fn();
    render(<Harness onResumed={onResumed} />);
    await waitFor(() => expect(onResumed).toHaveBeenCalledWith("execution-1"));
  });

  it("keeps denied work halted and checks again when the app regains focus", async () => {
    vi.mocked(invoke).mockResolvedValue([{ capabilityId: "calendar", state: "denied" }]);
    const onResumed = vi.fn();
    render(<Harness onResumed={onResumed} />);
    await waitFor(() => expect(invoke).toHaveBeenCalledTimes(1));
    act(() => window.dispatchEvent(new Event("focus")));
    await waitFor(() => expect(invoke).toHaveBeenCalledTimes(2));
    expect(onResumed).not.toHaveBeenCalled();
  });

  it("does not replay work the backend reports as already resumed", async () => {
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === "list_macos_permission_states") return [{ capabilityId: "calendar", state: "allowed" }];
      return { resumed: false, executionId: "execution-1", reason: "already_resumed" };
    });
    const onResumed = vi.fn();
    render(<Harness onResumed={onResumed} />);
    await waitFor(() => expect(invoke).toHaveBeenCalledTimes(2));
    expect(onResumed).not.toHaveBeenCalled();
  });
});
