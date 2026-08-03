import { useEffect, useRef } from "react";
import type { ChatTurnContext } from "@/lib/chatTurnContext";
import type {
  PrivateEgressConsentAttention,
  PrivateEgressConsentChoice,
} from "./PrivateEgressConsentCard";
import { useSessionScopedState, useStableEvent } from "./sessionScopedState";

type UsePrivateEgressConsentOptions = {
  activeSessionId: string;
  attentionStatus: string;
  resumingStatus: string;
  keptPrivateStatus: string;
  setSendingForSession: (sessionId: string, value: boolean) => void;
  setProcessingForSession: (sessionId: string, value: boolean) => void;
  setStatusForSession: (sessionId: string, value: string) => void;
};

export function usePrivateEgressConsent({
  activeSessionId,
  attentionStatus,
  resumingStatus,
  keptPrivateStatus,
  setSendingForSession,
  setProcessingForSession,
  setStatusForSession,
}: UsePrivateEgressConsentOptions) {
  const [attention, , setAttentionForSession, clearSessionAttention] =
    useSessionScopedState<PrivateEgressConsentAttention | null>(activeSessionId, null);
  const resolversRef = useRef(
    new Map<string, (choice: PrivateEgressConsentChoice) => void>(),
  );

  const requestChoice = useStableEvent(
    (context: ChatTurnContext, nextAttention: Omit<PrivateEgressConsentAttention, "sessionId">) =>
      new Promise<PrivateEgressConsentChoice>((resolve) => {
        resolversRef.current.get(context.sessionId)?.("keep_private");
        resolversRef.current.set(context.sessionId, resolve);
        setSendingForSession(context.sessionId, false);
        setProcessingForSession(context.sessionId, false);
        setStatusForSession(context.sessionId, attentionStatus);
        setAttentionForSession(context.sessionId, {
          ...nextAttention,
          sessionId: context.sessionId,
        });
      }),
  );

  const resolveChoice = useStableEvent((choice: PrivateEgressConsentChoice) => {
    if (!attention) return;
    const { sessionId } = attention;
    const resolve = resolversRef.current.get(sessionId);
    resolversRef.current.delete(sessionId);
    clearSessionAttention(sessionId);
    const shouldResume = choice === "send_once";
    setSendingForSession(sessionId, shouldResume);
    setProcessingForSession(sessionId, shouldResume);
    setStatusForSession(
      sessionId,
      shouldResume ? resumingStatus : keptPrivateStatus,
    );
    resolve?.(choice);
  });

  const cancelChoiceForSession = useStableEvent((sessionId: string) => {
    resolversRef.current.get(sessionId)?.("keep_private");
    resolversRef.current.delete(sessionId);
    clearSessionAttention(sessionId);
  });

  useEffect(
    () => () => {
      for (const resolve of resolversRef.current.values()) resolve("keep_private");
      resolversRef.current.clear();
    },
    [],
  );

  return {
    privateEgressConsentAttention: attention,
    requestPrivateEgressConsent: requestChoice,
    resolvePrivateEgressConsent: resolveChoice,
    cancelPrivateEgressConsentForSession: cancelChoiceForSession,
  };
}
