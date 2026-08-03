import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createChatTurnContext } from "@/lib/chatTurnContext";
import { invoke } from "@/lib/invoke";
import { readTurnRecovery, turnRecoveryIdentityKey } from "./turnRecoveryPersistence";
import { useAutoRouteTurnChoice } from "./useAutoRouteTurnChoice";

vi.mock("@/lib/invoke", () => ({ invoke: vi.fn() }));

const context = createChatTurnContext({
  turnId: "turn-301-recovery",
  generationToken: "generation-301-recovery",
  sessionId: "session-301-recovery",
  agentId: "agent-301",
  route: {
    providerId: "local_model",
    modelId: "gemma-4-E2B-it-qat-q4_0-gguf",
    dynamicRoutingEnabled: true,
    automatedWebGroundingEnabled: false,
  },
});

function options() {
  return {
    activeSessionId: context.sessionId,
    attentionStatus: "Auto-route needs attention",
    cancelledContent: "This task was cancelled before it finished.",
    choosingStatus: "Choosing a model",
    setSendingForSession: vi.fn(),
    setProcessingForSession: vi.fn(),
    setStatusForSession: vi.fn(),
    onResumePersistedTurn: vi.fn(async () => true),
    restoreEnabled: true,
    recoverableTurnIdentityKeys: new Set([turnRecoveryIdentityKey({
      sessionId: context.sessionId,
      rootTurnId: context.ancestry.rootTurnId,
      turnId: context.turnId,
      generationToken: context.generationToken,
    })]),
    terminalTurnIds: new Set<string>(),
  };
}

function requestRecovery(
  hook: { result: { current: ReturnType<typeof useAutoRouteTurnChoice> } },
  code: string,
) {
  return hook.result.current.requestAutoRouteTurnChoice(
    context, "local_model", context.route.modelId, "", { code },
  );
}

describe("Auto-route restart recovery actions", () => {
  beforeEach(() => {
    window.localStorage.clear();
    vi.mocked(invoke).mockReset();
  });
  afterEach(cleanup);

  it("persists the one-time continue-when-ready choice", async () => {
    const hook = renderHook(() => useAutoRouteTurnChoice(options()));
    act(() => { void requestRecovery(hook, "auto_route_classifier_not_ready"); });
    await waitFor(() => expect(hook.result.current.autoRouteAttention).not.toBeNull());

    await act(async () => {
      await hook.result.current.resolveAutoRouteTurnChoice("continue_when_ready");
    });

    expect(hook.result.current.autoRouteAttention?.continueWhenReady).toBe(true);
    expect(readTurnRecovery(context.sessionId, "auto_route")?.attention.continueWhenReady)
      .toBe(true);
  });

  it("durably cancels the exact restored turn before clearing its card", async () => {
    vi.mocked(invoke).mockResolvedValue(81);
    const first = renderHook(() => useAutoRouteTurnChoice(options()));
    act(() => { void requestRecovery(first, "classifier_inference_timeout"); });
    await waitFor(() => expect(first.result.current.autoRouteAttention).not.toBeNull());
    first.unmount();

    const restored = renderHook(() => useAutoRouteTurnChoice(options()));
    await waitFor(() => expect(restored.result.current.autoRouteAttention).not.toBeNull());
    await act(async () => {
      await restored.result.current.resolveAutoRouteTurnChoice("cancel");
    });

    expect(invoke).toHaveBeenCalledWith("cancel_saved_chat_turn", {
      request: {
        sessionId: context.sessionId,
        turnId: context.turnId,
        generationToken: context.generationToken,
        content: "This task was cancelled before it finished.",
      },
    });
    expect(restored.result.current.autoRouteAttention).toBeNull();
    expect(readTurnRecovery(context.sessionId, "auto_route")).toBeNull();
  });

  it("keeps restored work visible when durable cancellation fails", async () => {
    vi.mocked(invoke).mockRejectedValue(new Error("persistence unavailable"));
    const hook = renderHook(() => useAutoRouteTurnChoice(options()));
    act(() => { void requestRecovery(hook, "classifier_inference_timeout"); });
    await waitFor(() => expect(hook.result.current.autoRouteAttention).not.toBeNull());

    await expect(act(async () => {
      await hook.result.current.resolveAutoRouteTurnChoice("cancel");
    })).rejects.toThrow("persistence unavailable");
    expect(hook.result.current.autoRouteAttention).not.toBeNull();
    expect(readTurnRecovery(context.sessionId, "auto_route")).not.toBeNull();
  });

  it("does not restore the busy state after a persisted turn finishes resuming", async () => {
    const first = renderHook(() => useAutoRouteTurnChoice(options()));
    act(() => { void requestRecovery(first, "local_inference_cancelled"); });
    await waitFor(() => expect(first.result.current.autoRouteAttention).not.toBeNull());
    first.unmount();

    const restoredOptions = options();
    const restored = renderHook(() => useAutoRouteTurnChoice(restoredOptions));
    await waitFor(() => expect(restored.result.current.autoRouteAttention).not.toBeNull());
    await act(async () => {
      await restored.result.current.resolveAutoRouteTurnChoice("retry");
    });

    expect(restoredOptions.onResumePersistedTurn).toHaveBeenCalledOnce();
    expect(restoredOptions.setSendingForSession).not.toHaveBeenLastCalledWith(context.sessionId, true);
    expect(restoredOptions.setProcessingForSession).not.toHaveBeenLastCalledWith(context.sessionId, true);
  });
});
