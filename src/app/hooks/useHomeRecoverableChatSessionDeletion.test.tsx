import { act, renderHook } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ChatSession } from "@/lib/chatSessions";
import { useHomeRecoverableChatSessionDeletion } from "./useHomeRecoverableChatSessionDeletion";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

const sessions: ChatSession[] = [{
  id: "session-a",
  agentId: "agent-1",
  title: "Conversation",
  providerId: "provider-1",
  modelId: "model-1",
  projectId: null,
  webGroundingOverride: null,
  dynamicRoutingOverride: null,
  createdAtMs: 1,
  updatedAtMs: 1,
}];

beforeEach(() => {
  invokeMock.mockReset();
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("useHomeRecoverableChatSessionDeletion", () => {
  it("trusts committed native stage and Undo receipts without a fallible refresh gap", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "stage_chat_session_deletion") return true;
      if (command === "undo_chat_session_deletion") return true;
      throw new Error("session refresh unavailable");
    });
    const setError = vi.fn();
    const hook = renderHook(() => {
      const [currentSessions, setCurrentSessions] = useState(sessions);
      const [activeSessionId, setActiveSessionId] = useState("session-a");
      const deletion = useHomeRecoverableChatSessionDeletion({
        sessions: currentSessions,
        activeSessionId,
        setSessions: setCurrentSessions,
        setActiveSessionId,
        setChatSessionStateError: setError,
        t: (key) => key,
      });
      return { activeSessionId, currentSessions, ...deletion };
    });

    await act(async () => {
      expect(await hook.result.current.stageDelete("session-a")).toBe(true);
    });
    expect(hook.result.current.currentSessions).toEqual([]);
    expect(hook.result.current.recentlyDeletedSession).toEqual(sessions[0]);

    await act(async () => {
      expect(await hook.result.current.undoDelete()).toBe(true);
    });
    expect(hook.result.current.currentSessions).toEqual(sessions);
    expect(hook.result.current.activeSessionId).toBe("session-a");
    expect(invokeMock.mock.calls.map(([command]) => command)).toEqual([
      "stage_chat_session_deletion",
      "undo_chat_session_deletion",
    ]);
  });

  it("does not log an error when the same visible chat is deleted twice", async () => {
    vi.useFakeTimers();
    let confirmNativeStage: ((confirmed: boolean) => void) | undefined;
    const nativeStage = new Promise<boolean>((resolve) => {
      confirmNativeStage = resolve;
    });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "stage_chat_session_deletion") return nativeStage;
      if (command === "commit_chat_session_deletion") return true;
      throw new Error(`Unexpected command: ${command}`);
    });
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const setError = vi.fn();
    const hook = renderHook(() => {
      const [currentSessions, setCurrentSessions] = useState(sessions);
      const [activeSessionId, setActiveSessionId] = useState("session-a");
      const deletion = useHomeRecoverableChatSessionDeletion({
        sessions: currentSessions,
        activeSessionId,
        setSessions: setCurrentSessions,
        setActiveSessionId,
        setChatSessionStateError: setError,
        t: (key) => key,
      });
      return { currentSessions, ...deletion };
    });

    let firstDelete: Promise<boolean> | undefined;
    let repeatedDelete: Promise<boolean> | undefined;
    act(() => {
      firstDelete = hook.result.current.stageDelete("session-a");
      repeatedDelete = hook.result.current.stageDelete("session-a");
    });
    await act(async () => Promise.resolve());
    expect(invokeMock).toHaveBeenCalledOnce();

    await act(async () => {
      confirmNativeStage?.(true);
      expect(await Promise.all([firstDelete, repeatedDelete])).toEqual([true, true]);
    });
    expect(hook.result.current.currentSessions).toEqual([]);
    expect(consoleError).not.toHaveBeenCalled();
    expect(setError).not.toHaveBeenCalledWith("persistence_errors.chat_delete_failed");
  });

  it("surfaces an unconfirmed native commit as a real persistence failure", async () => {
    vi.useFakeTimers();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "stage_chat_session_deletion") return true;
      if (command === "commit_chat_session_deletion") return false;
      throw new Error(`Unexpected command: ${command}`);
    });
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const setError = vi.fn();
    const hook = renderHook(() => {
      const [currentSessions, setCurrentSessions] = useState(sessions);
      const [activeSessionId, setActiveSessionId] = useState("session-a");
      return useHomeRecoverableChatSessionDeletion({
        sessions: currentSessions,
        activeSessionId,
        setSessions: setCurrentSessions,
        setActiveSessionId,
        setChatSessionStateError: setError,
        t: (key) => key,
      });
    });

    await act(async () => {
      expect(await hook.result.current.stageDelete("session-a")).toBe(true);
    });
    await act(async () => vi.advanceTimersByTimeAsync(10_000));

    expect(consoleError).toHaveBeenCalledOnce();
    expect(setError).toHaveBeenCalledWith("persistence_errors.chat_delete_failed");
  });
});
