import type { ChatSession } from "@/lib/chatSessions";
import { useCallback, useMemo, useState, type Dispatch, type SetStateAction } from "react";

export function useProjectChatContext(
  sessions: ChatSession[],
  activeSessionId: string,
  setActiveSessionId: Dispatch<SetStateAction<string>>,
  openChat: () => void,
) {
  const [pendingProjectId, setPendingProjectId] = useState<string | null>(null);
  const handleSelectChatSession = useCallback((sessionId: string) => {
    setActiveSessionId(sessionId);
    setPendingProjectId(sessions.find((session) => session.id === sessionId)?.projectId ?? null);
  }, [sessions, setActiveSessionId]);
  const openProjectChat = useCallback((projectId: string) => {
    const existing = sessions.find((session) => session.projectId === projectId);
    setPendingProjectId(projectId);
    setActiveSessionId(existing?.id ?? "");
    openChat();
  }, [openChat, sessions, setActiveSessionId]);
  const clearProjectChatContext = useCallback(() => {
    setPendingProjectId(null);
    setActiveSessionId("");
  }, [setActiveSessionId]);
  const startGlobalChat = useCallback(() => {
    clearProjectChatContext();
    openChat();
  }, [clearProjectChatContext, openChat]);
  const activeChatProjectId = useMemo(
    () => sessions.find((session) => session.id === activeSessionId)?.projectId ?? pendingProjectId,
    [activeSessionId, pendingProjectId, sessions],
  );
  return {
    activeChatProjectId,
    clearProjectChatContext,
    handleSelectChatSession,
    openProjectChat,
    startGlobalChat,
  };
}
