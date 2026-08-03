import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ActiveAgentExecution } from "./agentExecutionState";
import { useRecoveryReceiptProjection } from "./useRecoveryReceiptProjection";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  isTauriRuntime: true,
}));

const content = JSON.stringify({
  schema: "oomu.agent_execution_recovery.v1",
  executionId: "execution-7",
  planId: "plan-7",
  code: "agent_execution_interrupted",
  boundary: "Mail",
  recoverable: true,
  recoveryAction: "resume_same_execution",
  message: "The exact approved Mail draft is ready to be restored.",
  changedState: "checkpoint_saved",
  context: {
    nextOperation: "draft_release_recovery_email",
    frozenArgumentSha256: "a".repeat(64),
  },
});

function execution(status: ActiveAgentExecution["status"]): ActiveAgentExecution {
  return {
    executionId: "execution-7",
    planId: "plan-7",
    sessionId: "session-7",
    status,
    logs: [],
    lastSeenId: 0,
    startedAtMs: 1,
  };
}

function projectionCalls() {
  return invokeMock.mock.calls.filter(([command]) =>
    command === "get_agent_execution_recovery_states"
  );
}

describe("useRecoveryReceiptProjection", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue([{
      executionId: "execution-7",
      planId: "plan-7",
      sessionId: "session-7",
      rootTurnId: "turn-7",
      failedTurnId: "turn-7",
      generationToken: "generation-7",
      status: "halted",
      terminalPhase: "restart_recovery_ready",
      terminalVerified: false,
      verifiedComplete: false,
    }]);
  });

  it("refreshes durable authority when execution becomes terminal without a new message ID", async () => {
    const { rerender } = renderHook(
      ({ activeExecution }) => useRecoveryReceiptProjection({
        activeExecution,
        activeSessionId: "session-7",
        completedRecoveryActionKeys: new Set(),
        messages: [{ id: 52, role: "assistant", content }],
      }),
      { initialProps: { activeExecution: execution("running") } },
    );
    await waitFor(() => expect(projectionCalls()).toHaveLength(1));

    rerender({ activeExecution: execution("halted") });

    await waitFor(() => expect(projectionCalls()).toHaveLength(2));
  });

  it("refreshes immediately when a terminal stream batch is observed", async () => {
    const { result } = renderHook(() => useRecoveryReceiptProjection({
      activeExecution: execution("running"),
      activeSessionId: "session-7",
      completedRecoveryActionKeys: new Set(),
      messages: [{ id: 52, role: "assistant", content }],
    }));
    await waitFor(() => expect(projectionCalls()).toHaveLength(1));

    act(() => result.current.refreshForTerminalBatch(
      "session-7",
      "execution-7",
      "halted",
    ));

    await waitFor(() => expect(projectionCalls()).toHaveLength(2));
  });

  it("does not requery durable authority for an unrelated appended message", async () => {
    const initialMessages: Array<{
      id: number;
      role: "user" | "assistant" | "system";
      content: string;
    }> = [{ id: 52, role: "assistant", content }];
    const { rerender } = renderHook(
      ({ messages }) => useRecoveryReceiptProjection({
        activeExecution: execution("running"),
        activeSessionId: "session-7",
        completedRecoveryActionKeys: new Set(),
        messages,
      }),
      { initialProps: { messages: initialMessages } },
    );
    await waitFor(() => expect(projectionCalls()).toHaveLength(1));

    rerender({
      messages: [
        ...initialMessages,
        { id: 53, role: "user", content: "Thanks." },
      ],
    });

    await waitFor(() => expect(projectionCalls()).toHaveLength(1));
  });

  it("isolates restart hydration and rejects a stale generation in another chat", async () => {
    const accepted = (generationToken: string) => ({
      id: 51,
      role: "user" as const,
      content: "Look online.",
      metadata: {
        turnId: "turn-7",
        rootTurnId: "turn-7",
        generationToken,
        turnState: "interrupted",
      },
    });
    const recoveredMessages = [accepted("generation-7"), {
      id: 52,
      role: "assistant" as const,
      content,
    }];
    const { result, rerender } = renderHook(
      ({ activeSessionId, messages }) => useRecoveryReceiptProjection({
        activeExecution: null,
        activeSessionId,
        completedRecoveryActionKeys: new Set(),
        messages,
      }),
      { initialProps: { activeSessionId: "session-7", messages: recoveredMessages } },
    );
    await waitFor(() => expect(result.current.snapshot.byExecutionId.size).toBe(1));

    rerender({
      activeSessionId: "session-8",
      messages: [accepted("generation-8"), recoveredMessages[1]],
    });
    await waitFor(() => expect(result.current.snapshot.sessionId).toBe("session-8"));
    expect(result.current.snapshot.byExecutionId.size).toBe(0);
  });
});
