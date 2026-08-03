import type { ChatSession } from "@/lib/chatSessions";
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type Dispatch,
  type SetStateAction,
} from "react";

const DEFAULT_UNDO_WINDOW_MS = 10_000;

type PendingChatSessionDeletion = {
  session: ChatSession;
  index: number;
  wasActive: boolean;
};

type RecoverableChatSessionDeletionOptions = {
  sessions: ChatSession[];
  activeSessionId: string;
  setSessions: Dispatch<SetStateAction<ChatSession[]>>;
  setActiveSessionId: Dispatch<SetStateAction<string>>;
  stageNativeDelete: (sessionId: string) => Promise<void>;
  undoNativeDelete: (sessionId: string) => Promise<void>;
  commitNativeDelete: (sessionId: string) => Promise<void>;
  onMutationFailure: (error: unknown) => void;
  undoWindowMs?: number;
};

function insertSessionAtOriginalPosition(
  sessions: ChatSession[],
  pending: PendingChatSessionDeletion,
) {
  if (sessions.some((session) => session.id === pending.session.id)) {
    return sessions;
  }
  const restored = [...sessions];
  restored.splice(Math.min(pending.index, restored.length), 0, pending.session);
  return restored;
}

function useCurrent<T>(value: T) {
  const ref = useRef(value);
  useEffect(() => {
    ref.current = value;
  }, [value]);
  return ref;
}

function usePendingDeletionLifecycle(
  commitNativeDelete: (sessionId: string) => Promise<void>,
  onMutationFailure: (error: unknown) => void,
  undoWindowMs: number,
) {
  const [recentlyDeletedSession, setRecentlyDeletedSession] =
    useState<ChatSession | null>(null);
  const pendingRef = useRef<PendingChatSessionDeletion | null>(null);
  const timerRef = useRef<number | null>(null);
  const commitNativeDeleteRef = useCurrent(commitNativeDelete);
  const onMutationFailureRef = useCurrent(onMutationFailure);
  const clearTimer = useCallback(() => {
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  }, []);
  const detachPending = useCallback(() => {
    clearTimer();
    const pending = pendingRef.current;
    pendingRef.current = null;
    setRecentlyDeletedSession(null);
    return pending;
  }, [clearTimer]);
  const commitPending = useCallback(async (pending: PendingChatSessionDeletion) => {
    try {
      await commitNativeDeleteRef.current(pending.session.id);
    } catch (error) {
      onMutationFailureRef.current(error);
    }
  }, [commitNativeDeleteRef, onMutationFailureRef]);
  const scheduleCommit = useCallback((pending: PendingChatSessionDeletion) => {
    clearTimer();
    timerRef.current = window.setTimeout(() => {
      if (pendingRef.current?.session.id !== pending.session.id) return;
      detachPending();
      void commitPending(pending);
    }, undoWindowMs);
  }, [clearTimer, commitPending, detachPending, undoWindowMs]);
  useEffect(() => () => {
    clearTimer();
    const pending = pendingRef.current;
    pendingRef.current = null;
    if (pending) void commitNativeDeleteRef.current(pending.session.id);
  }, [clearTimer, commitNativeDeleteRef]);
  return {
    clearTimer,
    commitPending,
    detachPending,
    pendingRef,
    recentlyDeletedSession,
    scheduleCommit,
    setRecentlyDeletedSession,
  };
}

function useCoalescedStageDelete(
  performStageDelete: (sessionId: string) => Promise<boolean>,
) {
  const inFlightStageDeletesRef = useRef(new Map<string, Promise<boolean>>());
  const stageDeleteQueueRef = useRef<Promise<void>>(Promise.resolve());
  return useCallback((sessionId: string) => {
    const existingDelete = inFlightStageDeletesRef.current.get(sessionId);
    if (existingDelete) return existingDelete;

    const deletion = stageDeleteQueueRef.current.then(() => performStageDelete(sessionId));
    stageDeleteQueueRef.current = deletion.then(() => undefined, () => undefined);
    inFlightStageDeletesRef.current.set(sessionId, deletion);
    const clearInFlightDelete = () => {
      if (inFlightStageDeletesRef.current.get(sessionId) === deletion) {
        inFlightStageDeletesRef.current.delete(sessionId);
      }
    };
    void deletion.then(clearInFlightDelete, clearInFlightDelete);
    return deletion;
  }, [performStageDelete]);
}

export function useRecoverableChatSessionDeletion({
  sessions,
  activeSessionId,
  setSessions,
  setActiveSessionId,
  stageNativeDelete,
  undoNativeDelete,
  commitNativeDelete,
  onMutationFailure,
  undoWindowMs = DEFAULT_UNDO_WINDOW_MS,
}: RecoverableChatSessionDeletionOptions) {
  const sessionsRef = useCurrent(sessions);
  const activeSessionIdRef = useCurrent(activeSessionId);
  const stageNativeDeleteRef = useCurrent(stageNativeDelete);
  const undoNativeDeleteRef = useCurrent(undoNativeDelete);
  const onMutationFailureRef = useCurrent(onMutationFailure);
  const confirmedDeletionIdsRef = useRef(new Set<string>());
  const lifecycle = usePendingDeletionLifecycle(
    commitNativeDelete,
    onMutationFailure,
    undoWindowMs,
  );
  const {
    clearTimer,
    commitPending,
    detachPending,
    pendingRef,
    recentlyDeletedSession,
    scheduleCommit,
    setRecentlyDeletedSession,
  } = lifecycle;

  const performStageDelete = useCallback(
    async (sessionId: string) => {
      const index = sessionsRef.current.findIndex((session) => session.id === sessionId);
      if (index === -1) {
        return confirmedDeletionIdsRef.current.has(sessionId);
      }

      const previousPending = detachPending();
      if (previousPending) {
        await commitPending(previousPending);
      }

      const session = sessionsRef.current[index];
      const pending = {
        session,
        index,
        wasActive: activeSessionIdRef.current === sessionId,
      };
      try {
        await stageNativeDeleteRef.current(sessionId);
      } catch (error) {
        onMutationFailureRef.current(error);
        return false;
      }
      confirmedDeletionIdsRef.current.add(sessionId);

      const remaining = sessionsRef.current.filter((entry) => entry.id !== sessionId);
      sessionsRef.current = remaining;
      setSessions(remaining);
      if (pending.wasActive) {
        const nextActiveSessionId = remaining[0]?.id ?? "";
        activeSessionIdRef.current = nextActiveSessionId;
        setActiveSessionId(nextActiveSessionId);
      }
      pendingRef.current = pending;
      setRecentlyDeletedSession(session);
      scheduleCommit(pending);
      return true;
    },
    [activeSessionIdRef, commitPending, detachPending, onMutationFailureRef, pendingRef, scheduleCommit, sessionsRef, setActiveSessionId, setRecentlyDeletedSession, setSessions, stageNativeDeleteRef],
  );
  const stageDelete = useCoalescedStageDelete(performStageDelete);

  const undoDelete = useCallback(async () => {
    const pending = pendingRef.current;
    if (!pending) {
      return false;
    }
    clearTimer();
    try {
      await undoNativeDeleteRef.current(pending.session.id);
    } catch (error) {
      onMutationFailureRef.current(error);
      scheduleCommit(pending);
      return false;
    }

    pendingRef.current = null;
    confirmedDeletionIdsRef.current.delete(pending.session.id);
    setRecentlyDeletedSession(null);
    const restored = insertSessionAtOriginalPosition(sessionsRef.current, pending);
    sessionsRef.current = restored;
    setSessions(restored);
    if (pending.wasActive) {
      activeSessionIdRef.current = pending.session.id;
      setActiveSessionId(pending.session.id);
    }
    return true;
  }, [activeSessionIdRef, clearTimer, onMutationFailureRef, pendingRef, scheduleCommit, sessionsRef, setActiveSessionId, setRecentlyDeletedSession, setSessions, undoNativeDeleteRef]);

  const excludePendingSession = useCallback((nextSessions: ChatSession[]) => {
    const pendingId = pendingRef.current?.session.id;
    return pendingId
      ? nextSessions.filter((session) => session.id !== pendingId)
      : nextSessions;
  }, [pendingRef]);

  return {
    excludePendingSession,
    recentlyDeletedSession,
    stageDelete,
    undoDelete,
  };
}
