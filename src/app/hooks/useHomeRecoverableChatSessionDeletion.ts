import { invoke } from "@tauri-apps/api/core";
import { useCallback, useRef, type Dispatch, type SetStateAction } from "react";
import type { ChatSession } from "@/lib/chatSessions";
import { useRecoverableChatSessionDeletion } from "./useRecoverableChatSessionDeletion";

type Translate = (key: string, params?: Record<string, string | number>) => string;

type HomeChatDeletionOptions = {
  sessions: ChatSession[];
  activeSessionId: string;
  setSessions: Dispatch<SetStateAction<ChatSession[]>>;
  setActiveSessionId: Dispatch<SetStateAction<string>>;
  setChatSessionStateError: Dispatch<SetStateAction<string>>;
  t: Translate;
};

export function useHomeRecoverableChatSessionDeletion({
  sessions,
  activeSessionId,
  setSessions,
  setActiveSessionId,
  setChatSessionStateError,
  t,
}: HomeChatDeletionOptions) {
  const nativeStagePromisesRef = useRef(new Map<string, Promise<void>>());
  const stageNativeDelete = useCallback((sessionId: string) => {
    const existingStage = nativeStagePromisesRef.current.get(sessionId);
    if (existingStage) return existingStage;

    const nativeStage = invoke<boolean>("stage_chat_session_deletion", {
      sessionId,
      session_id: sessionId,
    }).then((staged) => {
      if (!staged) throw new Error(t("chat.session_delete_unconfirmed"));
      setChatSessionStateError("");
    });
    nativeStagePromisesRef.current.set(sessionId, nativeStage);
    const clearNativeStage = () => {
      if (nativeStagePromisesRef.current.get(sessionId) === nativeStage) {
        nativeStagePromisesRef.current.delete(sessionId);
      }
    };
    void nativeStage.then(clearNativeStage, clearNativeStage);
    return nativeStage;
  }, [setChatSessionStateError, t]);
  const undoNativeDelete = useCallback(async (sessionId: string) => {
    const restored = await invoke<boolean>("undo_chat_session_deletion", {
      sessionId,
      session_id: sessionId,
    });
    if (!restored) throw new Error(t("chat.session_undo_unconfirmed"));
    setChatSessionStateError("");
  }, [setChatSessionStateError, t]);
  const commitNativeDelete = useCallback(async (sessionId: string) => {
    const committed = await invoke<boolean>("commit_chat_session_deletion", {
      sessionId,
      session_id: sessionId,
    });
    if (!committed) throw new Error(t("chat.session_delete_unconfirmed"));
  }, [t]);
  const onMutationFailure = useCallback((error: unknown) => {
    console.error("Recoverable chat deletion failed:", error);
    setChatSessionStateError("persistence_errors.chat_delete_failed");
  }, [setChatSessionStateError]);
  return useRecoverableChatSessionDeletion({
    sessions,
    activeSessionId,
    setSessions,
    setActiveSessionId,
    stageNativeDelete,
    undoNativeDelete,
    commitNativeDelete,
    onMutationFailure,
  });
}
