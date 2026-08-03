"use client";

import { useCallback, useEffect, useRef } from "react";
import type { ChatSession } from "@/lib/chatSessions";
import { invoke } from "@/lib/invoke";

type UseChatCompletionAttentionOptions = {
  activeSessionId: string;
  isNativeRuntime: boolean;
  isVisible: boolean;
  onSessionsChange: (sessions: ChatSession[]) => void;
  unreadCompletion: boolean;
};

export function useChatCompletionAttention({
  activeSessionId,
  isNativeRuntime,
  isVisible,
  onSessionsChange,
  unreadCompletion,
}: UseChatCompletionAttentionOptions) {
  const activeSessionIdRef = useRef(activeSessionId);
  const isVisibleRef = useRef(isVisible);
  const onSessionsChangeRef = useRef(onSessionsChange);

  useEffect(() => {
    activeSessionIdRef.current = activeSessionId;
    isVisibleRef.current = isVisible;
    onSessionsChangeRef.current = onSessionsChange;
  }, [activeSessionId, isVisible, onSessionsChange]);

  const publish = useCallback(async (sessionId: string, turnId: string) => {
    if (!isNativeRuntime || (isVisibleRef.current && activeSessionIdRef.current === sessionId)) {
      return;
    }
    try {
      const receipt = await invoke<{ bannerDelivered: boolean; newlyRecorded: boolean }>(
        "mark_chat_session_completion_unread",
        { request: { sessionId, turnId } },
      );
      if (receipt.newlyRecorded && !receipt.bannerDelivered) {
        console.warn("OOMU background response is unread, but macOS did not accept its banner.");
      }
      onSessionsChangeRef.current(await invoke<ChatSession[]>("list_chat_sessions"));
    } catch (error) {
      console.warn("Unable to publish background completion attention.", error);
    }
  }, [isNativeRuntime]);

  useEffect(() => {
    if (!isNativeRuntime || !isVisible || !activeSessionId || !unreadCompletion) return;
    void invoke("mark_chat_session_read", { sessionId: activeSessionId })
      .then(() => invoke<ChatSession[]>("list_chat_sessions"))
      .then((sessions) => onSessionsChangeRef.current(sessions))
      .catch((error) => console.warn("Unable to clear chat attention state.", error));
  }, [activeSessionId, isNativeRuntime, isVisible, unreadCompletion]);

  return publish;
}

type TerminalTurnResult = {
  status: "completed" | "failed" | "cancelled" | "escalated";
};

export async function finalizeTurnWithCompletionAttention<
  TContext,
  TResult extends TerminalTurnResult,
>(
  context: TContext,
  result: TResult,
  finalize: (context: TContext, result: TResult) => Promise<unknown>,
  publish: (sessionId: string, turnId: string) => Promise<void>,
  identity: { sessionId: string; turnId: string },
) {
  const finalized = await finalize(context, result).then(() => true).catch(() => false);
  if (finalized && result.status !== "cancelled") {
    await publish(identity.sessionId, identity.turnId);
  }
}
