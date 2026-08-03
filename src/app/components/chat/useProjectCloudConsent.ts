import { useEffect, useRef } from "react";
import type { ChatTurnContext } from "@/lib/chatTurnContext";
import type {
  ProjectCloudConsentAttention,
  ProjectCloudConsentChoice,
} from "./ProjectCloudConsentCard";
import { useSessionScopedState, useStableEvent } from "./sessionScopedState";

type UseProjectCloudConsentOptions = {
  activeSessionId: string;
  attentionStatus: string;
  resumingStatus: string;
  setSendingForSession: (sessionId: string, value: boolean) => void;
  setProcessingForSession: (sessionId: string, value: boolean) => void;
  setStatusForSession: (sessionId: string, value: string) => void;
};

export function useProjectCloudConsent({
  activeSessionId,
  attentionStatus,
  resumingStatus,
  setSendingForSession,
  setProcessingForSession,
  setStatusForSession,
}: UseProjectCloudConsentOptions) {
  const [attention, , setAttentionForSession, clearSessionAttention] =
    useSessionScopedState<ProjectCloudConsentAttention | null>(activeSessionId, null);
  const resolversRef = useRef(
    new Map<string, (choice: ProjectCloudConsentChoice) => void>(),
  );

  const requestChoice = useStableEvent(
    (context: ChatTurnContext, destination: string) =>
      new Promise<ProjectCloudConsentChoice>((resolve) => {
        resolversRef.current.get(context.sessionId)?.("cancel");
        resolversRef.current.set(context.sessionId, resolve);
        setSendingForSession(context.sessionId, false);
        setProcessingForSession(context.sessionId, false);
        setStatusForSession(context.sessionId, attentionStatus);
        setAttentionForSession(context.sessionId, {
          sessionId: context.sessionId,
          destination,
        });
      }),
  );

  const resolveChoice = useStableEvent((choice: ProjectCloudConsentChoice) => {
    if (!attention) return;
    const { sessionId } = attention;
    const resolve = resolversRef.current.get(sessionId);
    resolversRef.current.delete(sessionId);
    clearSessionAttention(sessionId);
    if (choice !== "cancel") {
      setSendingForSession(sessionId, true);
      setProcessingForSession(sessionId, true);
      setStatusForSession(sessionId, resumingStatus);
    }
    resolve?.(choice);
  });

  const cancelChoiceForSession = useStableEvent((sessionId: string) => {
    resolversRef.current.get(sessionId)?.("cancel");
    resolversRef.current.delete(sessionId);
    clearSessionAttention(sessionId);
  });

  useEffect(
    () => () => {
      for (const resolve of resolversRef.current.values()) resolve("cancel");
      resolversRef.current.clear();
    },
    [],
  );

  return {
    projectCloudConsentAttention: attention,
    requestProjectCloudConsent: requestChoice,
    resolveProjectCloudConsent: resolveChoice,
    cancelProjectCloudConsentForSession: cancelChoiceForSession,
  };
}
