import { useEffect, useRef } from "react";
import { act, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createChatTurnContext, type ChatTurnContext } from "@/lib/chatTurnContext";
import { useActiveChatTurns } from "./useActiveChatTurns";

const setters = {
  setActiveStreamId: vi.fn(),
  setSending: vi.fn(),
  setProcessing: vi.fn(),
  setMessages: vi.fn(),
  setStatus: vi.fn(),
};

function testTurn(sessionId: string) {
  return createChatTurnContext({
    turnId: `turn-${sessionId}`,
    generationToken: `generation-${sessionId}`,
    sessionId,
    agentId: "agent-1",
    route: {
      providerId: "local_model",
      modelId: "E2B",
      dynamicRoutingEnabled: false,
      automatedWebGroundingEnabled: false,
    },
  });
}

describe("useActiveChatTurns", () => {
  afterEach(() => vi.restoreAllMocks());

  it("keeps effect dependency shapes stable while the owner rerenders", () => {
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const { result, rerender } = renderHook(
      ({ sessionId, onStatus }) => {
        const activeTurnsRef = useRef(new Map<string, ChatTurnContext>());
        const turns = useActiveChatTurns<string>({ activeTurnsRef, ...setters });
        useEffect(() => void activeTurnsRef.current, []);
        useEffect(() => void activeTurnsRef.current.get(sessionId), [sessionId]);
        useEffect(() => void activeTurnsRef.current.size, [onStatus]);
        return turns;
      },
      { initialProps: { sessionId: "session-1", onStatus: setters.setStatus } },
    );

    const turn = testTurn("session-1");
    act(() => result.current.registerActiveTurn(turn, "stream-1"));
    const activeTurnsRef = result.current.activeTurnsRef;
    rerender({ sessionId: "session-2", onStatus: setters.setStatus });

    expect(result.current.activeTurnsRef).toBe(activeTurnsRef);
    expect(result.current.activeTurnForSession("session-1")).toEqual(turn);
    expect(consoleError.mock.calls.flat().join(" ")).not.toContain(
      "final argument passed to useEffect changed size",
    );
  });
});
