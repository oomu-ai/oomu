import { useRef } from "react";
import {
  chatTurnContextMatches,
  type ChatTurnContext,
} from "@/lib/chatTurnContext";
import type { SessionScopedStateUpdate } from "./sessionScopedState";

type ActiveChatTurnOptions<Message> = {
  activeTurnsRef: { current: Map<string, ChatTurnContext> };
  setActiveStreamId: (sessionId: string, streamId: string | null) => void;
  setSending: (sessionId: string, value: boolean) => void;
  setProcessing: (sessionId: string, value: boolean) => void;
  setMessages: (sessionId: string, update: SessionScopedStateUpdate<Message[]>) => void;
  setStatus: (sessionId: string, status: string) => void;
};

export function useActiveChatTurns<Message>(options: ActiveChatTurnOptions<Message>) {
  const { activeTurnsRef } = options;
  const activeStreamIdsRef = useRef(new Map<string, string>());
  const activeAssistantMessageIdsRef = useRef(new Map<string, number>());

  function activeTurnForSession(sessionId: string) {
    return activeTurnsRef.current.get(sessionId) ?? null;
  }

  function turnIsCurrent(context: ChatTurnContext) {
    const active = activeTurnForSession(context.sessionId);
    return Boolean(active && chatTurnContextMatches(active, context));
  }

  function registerActiveTurn(context: ChatTurnContext, streamId?: string | null) {
    activeTurnsRef.current.set(context.sessionId, context);
    if (!streamId) return;
    activeStreamIdsRef.current.set(context.sessionId, streamId);
    options.setActiveStreamId(context.sessionId, streamId);
  }

  function clearActiveTurn(context: ChatTurnContext) {
    if (!turnIsCurrent(context)) return;
    activeTurnsRef.current.delete(context.sessionId);
    activeStreamIdsRef.current.delete(context.sessionId);
    activeAssistantMessageIdsRef.current.delete(context.sessionId);
    options.setActiveStreamId(context.sessionId, null);
    options.setSending(context.sessionId, false);
    options.setProcessing(context.sessionId, false);
  }

  function updateTurnMessages(
    context: ChatTurnContext,
    update: SessionScopedStateUpdate<Message[]>,
  ) {
    if (!turnIsCurrent(context)) return false;
    options.setMessages(context.sessionId, update);
    return true;
  }

  function updateTurnStatus(context: ChatTurnContext, status: string) {
    if (!turnIsCurrent(context)) return false;
    options.setStatus(context.sessionId, status);
    return true;
  }

  return {
    activeTurnsRef,
    activeStreamIdsRef,
    activeAssistantMessageIdsRef,
    activeTurnForSession,
    registerActiveTurn,
    turnIsCurrent,
    clearActiveTurn,
    updateTurnMessages,
    updateTurnStatus,
  };
}
