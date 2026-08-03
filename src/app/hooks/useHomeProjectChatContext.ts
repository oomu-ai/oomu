import type { ChatSession } from "@/lib/chatSessions";
import { useEffect, useRef, type Dispatch, type SetStateAction } from "react";
import { useProjectChatContext } from "./useProjectChatContext";

export function useHomeProjectChatContext(
  sessions: ChatSession[],
  activeSessionId: string,
  setActiveSessionId: Dispatch<SetStateAction<string>>,
  openChat: () => void,
  globalChatRequestId: number,
) {
  const projectChat = useProjectChatContext(
    sessions,
    activeSessionId,
    setActiveSessionId,
    openChat,
  );
  const { activeChatProjectId, clearProjectChatContext } = projectChat;
  const handledGlobalChatRequestIdRef = useRef(globalChatRequestId);

  useEffect(() => {
    if (handledGlobalChatRequestIdRef.current === globalChatRequestId) return;
    handledGlobalChatRequestIdRef.current = globalChatRequestId;
    if (activeChatProjectId) clearProjectChatContext();
  }, [activeChatProjectId, clearProjectChatContext, globalChatRequestId]);

  return projectChat;
}
