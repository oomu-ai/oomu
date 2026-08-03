import { useRef } from "react";

export function usePendingChatSubmissions(activeSessionId: string, selectedAgentId: string) {
  const pendingSubmissionsRef = useRef(new Set<string>());

  return {
    scope(sessionId = activeSessionId) {
      return sessionId.trim() || `new:${selectedAgentId || "unselected"}`;
    },
    begin(scope: string) {
      if (pendingSubmissionsRef.current.has(scope)) return false;
      pendingSubmissionsRef.current.add(scope);
      return true;
    },
    end(scope: string) {
      pendingSubmissionsRef.current.delete(scope);
    },
    has(scope: string) {
      return pendingSubmissionsRef.current.has(scope);
    },
    removeSession(sessionId: string) {
      pendingSubmissionsRef.current.delete(sessionId);
    },
  };
}
