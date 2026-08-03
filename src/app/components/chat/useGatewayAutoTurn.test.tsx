import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useGatewayAutoTurn } from "./useGatewayAutoTurn";

const listeners = vi.hoisted(
  () => new Map<string, (event: { payload: unknown }) => void>(),
);
const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({
  invoke: invokeMock,
  isTauriRuntime: true,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (event: string, handler: (event: { payload: unknown }) => void) => {
    listeners.set(event, handler);
    return () => listeners.delete(event);
  }),
}));

describe("useGatewayAutoTurn", () => {
  beforeEach(() => {
    listeners.clear();
    invokeMock.mockReset();
    invokeMock.mockResolvedValue([]);
  });

  it.each(["completed", "failed"] as const)(
    "clears every activity state when a background turn is %s",
    async (status) => {
      const setExecuting = vi.fn();
      const setProcessing = vi.fn();
      const setSending = vi.fn();
      const refreshSessionMessages = vi.fn().mockResolvedValue(undefined);

      renderHook(() => useGatewayAutoTurn({
        translate: (key) => key,
        setExecuting,
        setProcessing,
        setSending,
        setStatus: vi.fn(),
        refreshSessionMessages,
        onSessionsChange: vi.fn(),
      }));

      await waitFor(() => expect(listeners.has("gateway://auto-turn")).toBe(true));
      act(() => {
        listeners.get("gateway://auto-turn")?.({
          payload: { sessionId: "session-1", status },
        });
      });

      expect(setExecuting).toHaveBeenCalledWith("session-1", false);
      expect(setProcessing).toHaveBeenCalledWith("session-1", false);
      expect(setSending).toHaveBeenCalledWith("session-1", false);
      if (status === "completed") {
        expect(refreshSessionMessages).toHaveBeenCalledWith("session-1");
      }
    },
  );
});
