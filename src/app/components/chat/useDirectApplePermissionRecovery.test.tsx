import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createChatTurnContext } from "@/lib/chatTurnContext";
import { invoke } from "@/lib/invoke";
import { readTurnRecovery } from "./turnRecoveryPersistence";
import { useDirectApplePermissionRecovery } from "./useDirectApplePermissionRecovery";

vi.mock("@/lib/invoke", () => ({
  invoke: vi.fn(),
  isTauriRuntime: true,
}));

const context = createChatTurnContext({
  turnId: "turn-calendar-301",
  generationToken: "generation-calendar-301",
  sessionId: "session-calendar-301",
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
  durableAttention: ReturnType<typeof permissionAttention> | null = null,
) {
  return {
    activeSessionId: context.sessionId,
    attentionStatus: "Calendar access needed",
    choosingStatus: "Continuing",
    durableAttention,
    onResumePersistedTurn,
    restoreEnabled: true,
    terminalTurnIds,
    setProcessingForSession: vi.fn(),
    setSendingForSession: vi.fn(),
    setStatusForSession: vi.fn(),
  };
}

function permissionAttention() {
  return {
    sessionId: context.sessionId,
    rootTurnId: context.ancestry.rootTurnId,
    turnId: context.turnId,
    generationToken: context.generationToken,
    boundary: "macos_permission_broker",
    code: "calendar_permission_denied",
    descriptor: { capabilityId: "calendar", state: "denied" as const },
  };
}

describe("direct Apple permission recovery", () => {
  beforeEach(() => {
    window.localStorage.clear();
    vi.clearAllMocks();
  });
  afterEach(cleanup);

  it("retries the same live accepted turn after the user checks access", async () => {
    let terminalTurnIds = new Set<string>();
    const hook = renderHook(() => useDirectApplePermissionRecovery(
      options(undefined, terminalTurnIds),
    ));
    let resultPromise: Promise<"retry" | "cancel"> | null = null;
    act(() => {
      resultPromise = hook.result.current.requestDirectApplePermissionRecovery(
        context,
        "calendar",
        { code: "calendar_permission_denied" },
      );
    });
    await waitFor(() => expect(hook.result.current.directApplePermissionAttention)
      .toMatchObject({ turnId: context.turnId, descriptor: { capabilityId: "calendar" } }));

    await act(async () => {
      await hook.result.current.directApplePermissionActions.onCheck();
    });
    await expect(resultPromise).resolves.toBe("retry");
    expect(readTurnRecovery(context.sessionId, "apple_permission")).not.toBeNull();
    terminalTurnIds = new Set([context.turnId]);
    hook.rerender();
    await waitFor(() => expect(readTurnRecovery(context.sessionId, "apple_permission")).toBeNull());
  });

  it("restores the permission card from the durable turn without local storage", async () => {
    const resume = vi.fn(async () => true);
    const restored = renderHook(() => useDirectApplePermissionRecovery(
      options(resume, new Set(), permissionAttention()),
    ));
    await waitFor(() => expect(restored.result.current.directApplePermissionAttention?.turnId)
      .toBe(context.turnId));
    await act(async () => {
      await restored.result.current.directApplePermissionActions.onCheck();
    });

    expect(resume).toHaveBeenCalledTimes(1);
    expect(resume).toHaveBeenCalledWith(expect.objectContaining({
      sessionId: context.sessionId,
      turnId: context.turnId,
      generationToken: context.generationToken,
    }));
  });

  it("durably cancels a restored turn before removing its recovery card", async () => {
    let finishCancel: (() => void) | null = null;
    vi.mocked(invoke).mockImplementation(() => new Promise((resolve) => {
      finishCancel = () => resolve({ cancelled: true, receiptId: "cancel-receipt-301" });
    }));
    const restored = renderHook(() => useDirectApplePermissionRecovery(
      options(undefined, new Set(), permissionAttention()),
    ));
    await waitFor(() => expect(restored.result.current.directApplePermissionAttention?.turnId)
      .toBe(context.turnId));
    let cancellation: Promise<void> | undefined;
    act(() => {
      cancellation = restored.result.current.directApplePermissionActions.onCancel();
    });
    expect(restored.result.current.directApplePermissionAttention?.turnId).toBe(context.turnId);
    expect(invoke).toHaveBeenCalledWith("cancel_permission_recovery_turn", {
      request: {
        sessionId: context.sessionId,
        turnId: context.turnId,
        generationToken: context.generationToken,
        capabilityId: "calendar",
      },
    });
    await act(async () => {
      finishCancel?.();
      await cancellation;
    });
    expect(restored.result.current.directApplePermissionAttention).toBeNull();
  });

  it("does not turn a non-permission tool failure into a permission prompt", () => {
    const hook = renderHook(() => useDirectApplePermissionRecovery(options()));
    expect(hook.result.current.requestDirectApplePermissionRecovery(
      context,
      "calendar",
      { code: "calendar_data_invalid" },
    )).toBeNull();
    expect(hook.result.current.directApplePermissionAttention).toBeNull();
  });

  it("does not make local storage a prerequisite for the durable recovery card", async () => {
    const setItem = vi.spyOn(Storage.prototype, "setItem")
      .mockImplementation(() => { throw new Error("storage unavailable"); });
    const hook = renderHook(() => useDirectApplePermissionRecovery(options()));
    act(() => {
      void hook.result.current.requestDirectApplePermissionRecovery(
        context, "calendar", { code: "calendar_permission_denied" },
      );
    });
    await waitFor(() => expect(hook.result.current.directApplePermissionAttention?.turnId)
      .toBe(context.turnId));
    setItem.mockRestore();
  });
});

describe("direct Apple timeout recovery", () => {
  beforeEach(() => {
    window.localStorage.clear();
    vi.clearAllMocks();
  });
  afterEach(cleanup);

  it("keeps a native Apple read timeout recoverable for every supported app", async () => {
    const hook = renderHook(() => useDirectApplePermissionRecovery(options()));
    let resultPromise: Promise<"retry" | "cancel"> | null = null;
    act(() => {
      resultPromise = hook.result.current.requestDirectApplePermissionRecovery(
        context, "notes", { code: "timeout" },
      );
    });
    await waitFor(() => expect(hook.result.current.directApplePermissionAttention)
      .toMatchObject({
        code: "notes_permission_timeout",
        descriptor: { capabilityId: "notes", state: "timeout" },
      }));
    await act(async () => {
      await hook.result.current.directApplePermissionActions.onCancel();
    });
    await expect(resultPromise).resolves.toBe("cancel");
  });
});
