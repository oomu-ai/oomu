import { act, renderHook } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ChatSession } from "@/lib/chatSessions";
import { useRecoverableChatSessionDeletion } from "./useRecoverableChatSessionDeletion";

const sessions: ChatSession[] = [
  {
    id: "session-a",
    agentId: "agent-1",
    title: "First conversation",
    providerId: "provider-1",
    modelId: "model-1",
    projectId: null,
    webGroundingOverride: null,
    dynamicRoutingOverride: null,
    createdAtMs: 1,
    updatedAtMs: 1,
  },
  {
    id: "session-b",
    agentId: "agent-1",
    title: "Second conversation",
    providerId: "provider-1",
    modelId: "model-1",
    projectId: null,
    webGroundingOverride: null,
    dynamicRoutingOverride: null,
    createdAtMs: 2,
    updatedAtMs: 2,
  },
];

function renderDeletionHook(overrides: {
  stageNativeDelete?: (sessionId: string) => Promise<void>;
  undoNativeDelete?: (sessionId: string) => Promise<void>;
  commitNativeDelete?: (sessionId: string) => Promise<void>;
} = {}) {
  const stageNativeDelete = vi.fn(overrides.stageNativeDelete ?? (async () => undefined));
  const undoNativeDelete = vi.fn(overrides.undoNativeDelete ?? (async () => undefined));
  const commitNativeDelete = vi.fn(overrides.commitNativeDelete ?? (async () => undefined));
  const onMutationFailure = vi.fn();
  const hook = renderHook(() => {
    const [currentSessions, setCurrentSessions] = useState(sessions);
    const [activeSessionId, setActiveSessionId] = useState("session-a");
    const deletion = useRecoverableChatSessionDeletion({
      sessions: currentSessions,
      activeSessionId,
      setSessions: setCurrentSessions,
      setActiveSessionId,
      stageNativeDelete,
      undoNativeDelete,
      commitNativeDelete,
      onMutationFailure,
    });
    return { activeSessionId, currentSessions, ...deletion };
  });
  return {
    commitNativeDelete,
    hook,
    onMutationFailure,
    stageNativeDelete,
    undoNativeDelete,
  };
}

afterEach(() => {
  vi.useRealTimers();
});

describe("useRecoverableChatSessionDeletion staging", () => {
  it("keeps the chat visible until native staging confirms revocation", async () => {
    vi.useFakeTimers();
    let confirmNativeStage: (() => void) | undefined;
    const nativeStage = new Promise<void>((resolve) => {
      confirmNativeStage = resolve;
    });
    const { hook } = renderDeletionHook({
      stageNativeDelete: async () => nativeStage,
    });

    let deletion: Promise<boolean> | undefined;
    act(() => {
      deletion = hook.result.current.stageDelete("session-a");
    });
    expect(hook.result.current.currentSessions).toEqual(sessions);
    expect(hook.result.current.recentlyDeletedSession).toBeNull();

    await act(async () => {
      confirmNativeStage?.();
      expect(await deletion).toBe(true);
    });
    expect(hook.result.current.currentSessions).toEqual([sessions[1]]);
    expect(hook.result.current.recentlyDeletedSession).toEqual(sessions[0]);
  });

  it("coalesces repeated delete requests around one native confirmation", async () => {
    vi.useFakeTimers();
    let confirmNativeStage: (() => void) | undefined;
    const nativeStage = new Promise<void>((resolve) => {
      confirmNativeStage = resolve;
    });
    const { hook, onMutationFailure, stageNativeDelete } = renderDeletionHook({
      stageNativeDelete: async () => nativeStage,
    });

    let firstDelete: Promise<boolean> | undefined;
    let repeatedDelete: Promise<boolean> | undefined;
    act(() => {
      firstDelete = hook.result.current.stageDelete("session-a");
      repeatedDelete = hook.result.current.stageDelete("session-a");
    });
    expect(repeatedDelete).toBe(firstDelete);

    await act(async () => Promise.resolve());
    expect(stageNativeDelete).toHaveBeenCalledOnce();
    expect(hook.result.current.currentSessions).toEqual(sessions);

    await act(async () => {
      confirmNativeStage?.();
      expect(await Promise.all([firstDelete, repeatedDelete])).toEqual([true, true]);
    });
    expect(hook.result.current.currentSessions).toEqual([sessions[1]]);
    expect(onMutationFailure).not.toHaveBeenCalled();
  });

  it("reports one genuine native failure to every coalesced delete caller", async () => {
    const error = new Error("native staging failed");
    const { hook, onMutationFailure, stageNativeDelete } = renderDeletionHook({
      stageNativeDelete: async () => Promise.reject(error),
    });

    let firstDelete: Promise<boolean> | undefined;
    let repeatedDelete: Promise<boolean> | undefined;
    act(() => {
      firstDelete = hook.result.current.stageDelete("session-a");
      repeatedDelete = hook.result.current.stageDelete("session-a");
    });
    await act(async () => {
      expect(await Promise.all([firstDelete, repeatedDelete])).toEqual([false, false]);
    });

    expect(stageNativeDelete).toHaveBeenCalledOnce();
    expect(onMutationFailure).toHaveBeenCalledOnce();
    expect(onMutationFailure).toHaveBeenCalledWith(error);
    expect(hook.result.current.currentSessions).toEqual(sessions);
  });

  it("serializes rapid deletions so each native stage keeps its own Undo lifecycle", async () => {
    vi.useFakeTimers();
    let confirmFirstStage: (() => void) | undefined;
    const firstNativeStage = new Promise<void>((resolve) => {
      confirmFirstStage = resolve;
    });
    const { commitNativeDelete, hook, stageNativeDelete } = renderDeletionHook({
      stageNativeDelete: async (sessionId) => {
        if (sessionId === "session-a") await firstNativeStage;
      },
    });

    let firstDelete: Promise<boolean> | undefined;
    let secondDelete: Promise<boolean> | undefined;
    act(() => {
      firstDelete = hook.result.current.stageDelete("session-a");
      secondDelete = hook.result.current.stageDelete("session-b");
    });
    await act(async () => Promise.resolve());
    expect(stageNativeDelete).toHaveBeenCalledTimes(1);
    expect(stageNativeDelete).toHaveBeenCalledWith("session-a");

    await act(async () => {
      confirmFirstStage?.();
      expect(await Promise.all([firstDelete, secondDelete])).toEqual([true, true]);
    });
    expect(commitNativeDelete).toHaveBeenCalledOnce();
    expect(commitNativeDelete).toHaveBeenCalledWith("session-a");
    expect(stageNativeDelete).toHaveBeenNthCalledWith(2, "session-b");
    expect(commitNativeDelete.mock.invocationCallOrder[0]).toBeLessThan(
      stageNativeDelete.mock.invocationCallOrder[1],
    );
    expect(hook.result.current.currentSessions).toEqual([]);
    expect(hook.result.current.recentlyDeletedSession).toEqual(sessions[1]);
  });
});

describe("useRecoverableChatSessionDeletion lifecycle", () => {
  it("revokes natively before hiding and restores the same session on Undo", async () => {
    vi.useFakeTimers();
    const { commitNativeDelete, hook, stageNativeDelete, undoNativeDelete } =
      renderDeletionHook();

    await act(async () => {
      expect(await hook.result.current.stageDelete("session-a")).toBe(true);
    });
    expect(stageNativeDelete).toHaveBeenCalledWith("session-a");
    expect(hook.result.current.currentSessions).toEqual([sessions[1]]);
    expect(hook.result.current.activeSessionId).toBe("session-b");
    expect(hook.result.current.recentlyDeletedSession).toEqual(sessions[0]);

    await act(async () => {
      expect(await hook.result.current.undoDelete()).toBe(true);
    });
    expect(undoNativeDelete).toHaveBeenCalledWith("session-a");
    expect(hook.result.current.currentSessions).toEqual(sessions);
    expect(hook.result.current.activeSessionId).toBe("session-a");

    await act(async () => vi.advanceTimersByTimeAsync(10_000));
    expect(commitNativeDelete).not.toHaveBeenCalled();
  });

  it("commits the recoverable archive only after the Undo window", async () => {
    vi.useFakeTimers();
    const { commitNativeDelete, hook } = renderDeletionHook();
    await act(async () => {
      await hook.result.current.stageDelete("session-a");
      await vi.advanceTimersByTimeAsync(9_999);
    });
    expect(commitNativeDelete).not.toHaveBeenCalled();

    await act(async () => vi.advanceTimersByTimeAsync(1));
    expect(commitNativeDelete).toHaveBeenCalledOnce();
    expect(commitNativeDelete).toHaveBeenCalledWith("session-a");
    expect(hook.result.current.recentlyDeletedSession).toBeNull();
  });

  it("commits the first archive before staging a second deletion", async () => {
    vi.useFakeTimers();
    const { commitNativeDelete, hook, stageNativeDelete } = renderDeletionHook();
    await act(async () => {
      expect(await hook.result.current.stageDelete("session-a")).toBe(true);
      expect(await hook.result.current.stageDelete("session-b")).toBe(true);
    });
    expect(commitNativeDelete).toHaveBeenCalledOnce();
    expect(commitNativeDelete).toHaveBeenCalledWith("session-a");
    expect(stageNativeDelete).toHaveBeenNthCalledWith(2, "session-b");
    expect(commitNativeDelete.mock.invocationCallOrder[0]).toBeLessThan(
      stageNativeDelete.mock.invocationCallOrder[1],
    );
    expect(hook.result.current.recentlyDeletedSession).toEqual(sessions[1]);
  });

  it("keeps a staged native deletion hidden across list refetches", async () => {
    vi.useFakeTimers();
    const { hook } = renderDeletionHook();
    await act(async () => {
      await hook.result.current.stageDelete("session-a");
    });
    expect(hook.result.current.excludePendingSession(sessions)).toEqual([sessions[1]]);
  });

  it("does not hide the chat when native staging fails", async () => {
    const error = new Error("native staging failed");
    const { hook, onMutationFailure } = renderDeletionHook({
      stageNativeDelete: async () => Promise.reject(error),
    });
    await act(async () => {
      expect(await hook.result.current.stageDelete("session-a")).toBe(false);
    });
    expect(hook.result.current.currentSessions).toEqual(sessions);
    expect(hook.result.current.recentlyDeletedSession).toBeNull();
    expect(onMutationFailure).toHaveBeenCalledWith(error);
  });

  it("keeps Undo available when native restoration fails", async () => {
    vi.useFakeTimers();
    const error = new Error("native restore failed");
    const { hook, onMutationFailure } = renderDeletionHook({
      undoNativeDelete: async () => Promise.reject(error),
    });
    await act(async () => {
      await hook.result.current.stageDelete("session-a");
      expect(await hook.result.current.undoDelete()).toBe(false);
    });
    expect(hook.result.current.currentSessions).toEqual([sessions[1]]);
    expect(hook.result.current.recentlyDeletedSession).toEqual(sessions[0]);
    expect(onMutationFailure).toHaveBeenCalledWith(error);
  });
});
