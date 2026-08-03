import { act, renderHook } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import type { ChatSession } from "@/lib/chatSessions";
import { useHomeProjectChatContext } from "./useHomeProjectChatContext";
import { useProjectChatContext } from "./useProjectChatContext";

const sessions: ChatSession[] = [
  {
    id: "project-session",
    agentId: "agent-1",
    title: "Project conversation",
    providerId: "provider-1",
    modelId: "model-1",
    projectId: "project-1",
    webGroundingOverride: null,
    dynamicRoutingOverride: null,
    createdAtMs: 1,
    updatedAtMs: 1,
  },
  {
    id: "global-session",
    agentId: "agent-1",
    title: "Global conversation",
    providerId: "provider-1",
    modelId: "model-1",
    webGroundingOverride: null,
    dynamicRoutingOverride: null,
    createdAtMs: 2,
    updatedAtMs: 2,
  },
];

describe("useProjectChatContext", () => {
  it("clears Project scope when the shell requests global chat", () => {
    const { result, rerender } = renderHook(
      ({ requestId }) => {
        const [activeSessionId, setActiveSessionId] = useState("project-session");
        return {
          activeSessionId,
          ...useHomeProjectChatContext(
            sessions,
            activeSessionId,
            setActiveSessionId,
            vi.fn(),
            requestId,
          ),
        };
      },
      { initialProps: { requestId: 1 } },
    );

    expect(result.current.activeChatProjectId).toBe("project-1");
    rerender({ requestId: 2 });
    expect(result.current.activeSessionId).toBe("");
    expect(result.current.activeChatProjectId).toBeNull();
  });

  it("clears both the pending Project and active Project session for global chat", () => {
    const openChat = vi.fn();
    const { result } = renderHook(() => {
      const [activeSessionId, setActiveSessionId] = useState("");
      return {
        activeSessionId,
        ...useProjectChatContext(
          sessions,
          activeSessionId,
          setActiveSessionId,
          openChat,
        ),
      };
    });

    act(() => result.current.openProjectChat("project-1"));
    expect(result.current.activeSessionId).toBe("project-session");
    expect(result.current.activeChatProjectId).toBe("project-1");

    act(() => result.current.startGlobalChat());
    expect(result.current.activeSessionId).toBe("");
    expect(result.current.activeChatProjectId).toBeNull();
    expect(openChat).toHaveBeenCalledTimes(2);
  });

  it("clears pending Project scope when a global conversation is selected", () => {
    const { result } = renderHook(() => {
      const [activeSessionId, setActiveSessionId] = useState("");
      return {
        activeSessionId,
        ...useProjectChatContext(
          sessions,
          activeSessionId,
          setActiveSessionId,
          vi.fn(),
        ),
      };
    });

    act(() => result.current.openProjectChat("project-1"));
    act(() => result.current.handleSelectChatSession("global-session"));

    expect(result.current.activeSessionId).toBe("global-session");
    expect(result.current.activeChatProjectId).toBeNull();
  });
});
