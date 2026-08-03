import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createChatTurnContext } from "@/lib/chatTurnContext";
import { invoke } from "@/lib/invoke";
import { readTurnRecovery, turnRecoveryIdentityKey } from "./turnRecoveryPersistence";
import { useAutoRouteTurnChoice } from "./useAutoRouteTurnChoice";

vi.mock("@/lib/invoke", () => ({ invoke: vi.fn() }));

const context = createChatTurnContext({
  turnId: "turn-301",
  generationToken: "generation-301",
  sessionId: "session-301",
  agentId: "agent-301",
  route: {
    providerId: "local_model",
    modelId: "gemma-4-E2B-it-qat-q4_0-gguf",
    dynamicRoutingEnabled: true,
    automatedWebGroundingEnabled: false,
  },
});

function options(
  onResumePersistedTurn = vi.fn(async () => true),
  terminalTurnIds: ReadonlySet<string> = new Set(),
) {
  return {
    activeSessionId: context.sessionId,
    attentionStatus: "Auto-route needs attention",
    cancelledContent: "This task was cancelled before it finished.",
    choosingStatus: "Choosing a model",
    setSendingForSession: vi.fn(),
    setProcessingForSession: vi.fn(),
    setStatusForSession: vi.fn(),
    onResumePersistedTurn,
    restoreEnabled: true,
    recoverableTurnIdentityKeys: new Set([turnRecoveryIdentityKey({
      sessionId: context.sessionId,
      rootTurnId: context.ancestry.rootTurnId,
      turnId: context.turnId,
      generationToken: context.generationToken,
    })]),
    terminalTurnIds,
  };
}

describe("durable Auto-route turn choice", () => {
  beforeEach(() => {
    window.localStorage.clear();
    vi.mocked(invoke).mockReset();
  });
  afterEach(cleanup);

  it("restores a stopped turn after remount and resumes it exactly once", async () => {
    const first = renderHook(() => useAutoRouteTurnChoice(options()));
    act(() => {
      void first.result.current.requestAutoRouteTurnChoice(
        context,
        "local_model",
        "gemma-4-E2B-it-qat-q4_0-gguf",
        "gemini-3.5-flash",
        { code: "classifier_inference_timeout" },
      );
    });
    await waitFor(() => expect(first.result.current.autoRouteAttention?.turnId)
      .toBe(context.turnId));
    expect(readTurnRecovery(context.sessionId, "auto_route")).not.toBeNull();
    first.unmount();

    const resume = vi.fn(async () => true);
    let terminalTurnIds = new Set<string>();
    const restored = renderHook(() => useAutoRouteTurnChoice(options(resume, terminalTurnIds)));
    await waitFor(() => expect(restored.result.current.autoRouteAttention?.generationToken)
      .toBe(context.generationToken));
    await act(async () => {
      await restored.result.current.resolveAutoRouteTurnChoice("retry");
    });

    expect(resume).toHaveBeenCalledTimes(1);
    expect(resume).toHaveBeenCalledWith(
      expect.objectContaining({ turnId: context.turnId, generationToken: context.generationToken }),
      "retry",
    );
    expect(readTurnRecovery(context.sessionId, "auto_route")).not.toBeNull();
    terminalTurnIds = new Set([context.turnId]);
    restored.rerender();
    await waitFor(() => expect(readTurnRecovery(context.sessionId, "auto_route")).toBeNull());
  });

  it("does not discard saved recovery when replay cannot be accepted", async () => {
    const first = renderHook(() => useAutoRouteTurnChoice(options()));
    act(() => {
      void first.result.current.requestAutoRouteTurnChoice(
        context, "local_model", context.route.modelId, "", { code: "classifier_not_ready" },
      );
    });
    await waitFor(() => expect(first.result.current.autoRouteAttention).not.toBeNull());
    first.unmount();

    const restored = renderHook(() => useAutoRouteTurnChoice(options(vi.fn(async () => false))));
    await waitFor(() => expect(restored.result.current.autoRouteAttention).not.toBeNull());
    await expect(act(async () => {
      await restored.result.current.resolveAutoRouteTurnChoice("retry");
    })).rejects.toThrow("auto_route_saved_turn_unavailable");
    expect(readTurnRecovery(context.sessionId, "auto_route")).not.toBeNull();
  });

  it("does not show a recovery card when its identity could not be saved", async () => {
    const setItem = vi.spyOn(Storage.prototype, "setItem")
      .mockImplementation(() => { throw new Error("storage unavailable"); });
    const hook = renderHook(() => useAutoRouteTurnChoice(options()));
    await expect(hook.result.current.requestAutoRouteTurnChoice(
      context, "local_model", context.route.modelId, "", { code: "classifier_not_ready" },
    )).rejects.toThrow("chat_turn_persistence_failed");
    expect(hook.result.current.autoRouteAttention).toBeNull();
    setItem.mockRestore();
  });

  it("repairs only the stopped turn and generation shown to the user", async () => {
    vi.mocked(invoke).mockResolvedValue({ status: "ready" });
    const hook = renderHook(() => useAutoRouteTurnChoice(options()));
    let choice: Promise<string> | undefined;
    act(() => {
      choice = hook.result.current.requestAutoRouteTurnChoice(
        context,
        "local_model",
        "missing-explicit-model",
        "",
        { code: "auto_route_session_local_model_unavailable" },
        "local_model",
        "gemma-4-E2B-it-qat-q4_0-gguf",
      );
    });
    await waitFor(() => expect(hook.result.current.autoRouteAttention).not.toBeNull());

    await act(async () => {
      await hook.result.current.resolveAutoRouteTurnChoice("repair_model");
    });

    expect(invoke).toHaveBeenCalledWith("repair_auto_route_session_baseline", {
      request: {
        sessionId: context.sessionId,
        turnId: context.turnId,
        generationToken: context.generationToken,
        localProviderId: "local_model",
        localModelId: "gemma-4-E2B-it-qat-q4_0-gguf",
      },
    });
    await expect(choice).resolves.toBe("retry");
  });

});
