import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useChatCompletionAttention } from "./useChatCompletionAttention";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

describe("useChatCompletionAttention", () => {
  beforeEach(() => invokeMock.mockReset());

  it("publishes once for a terminal turn when Chat is hidden", async () => {
    const onSessionsChange = vi.fn();
    invokeMock.mockImplementation((command: string) => {
      if (command === "mark_chat_session_completion_unread") {
        return { bannerDelivered: true, newlyRecorded: true };
      }
      if (command === "list_chat_sessions") return [];
      return Promise.resolve(null);
    });
    const { result, rerender } = renderHook(
      ({ isVisible }) => useChatCompletionAttention({
        activeSessionId: "session-1",
        isNativeRuntime: true,
        isVisible,
        onSessionsChange,
        unreadCompletion: false,
      }),
      { initialProps: { isVisible: true } },
    );

    rerender({ isVisible: false });
    await act(() => result.current("session-1", "turn-1"));

    expect(invokeMock).toHaveBeenCalledWith("mark_chat_session_completion_unread", {
      request: { sessionId: "session-1", turnId: "turn-1" },
    });
    await waitFor(() => expect(onSessionsChange).toHaveBeenCalledWith([]));
  });

  it("clears unread attention only while its session is visible", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_chat_sessions") return [];
      return Promise.resolve(null);
    });
    const { rerender } = renderHook(
      ({ isVisible }) => useChatCompletionAttention({
        activeSessionId: "session-1",
        isNativeRuntime: true,
        isVisible,
        onSessionsChange: vi.fn(),
        unreadCompletion: true,
      }),
      { initialProps: { isVisible: false } },
    );
    expect(invokeMock).not.toHaveBeenCalledWith("mark_chat_session_read", expect.anything());
    rerender({ isVisible: true });
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "mark_chat_session_read", { sessionId: "session-1" },
    ));
  });
});
