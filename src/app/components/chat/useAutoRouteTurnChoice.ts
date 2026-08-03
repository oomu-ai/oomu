import { useEffect, useRef, type MutableRefObject } from "react";
import { invoke } from "@/lib/invoke";
import type { ChatTurnContext } from "@/lib/chatTurnContext";
import type {
  AutoRouteAttention,
  AutoRouteRecoveryAction,
  AutoRouteTurnChoice,
} from "./AutoRouteAttentionCard";
import { autoRouteRecoveryEvidence } from "./autoRouteRecovery";
import { normalizeAutoRouteSessionReadiness } from "./autoRouteReadiness";
import { useSessionScopedState, useStableEvent } from "./sessionScopedState";
import {
  clearTurnRecovery,
  persistTurnRecovery,
  readMatchingTurnRecovery,
} from "./turnRecoveryPersistence";

const noTerminalTurns = new Set<string>();
const noRecoverableTurnIdentities = new Set<string>();
const recoveryKey = (value: { turnId: string; generationToken: string }) =>
  `${value.turnId}:${value.generationToken}`;

type UseAutoRouteTurnChoiceOptions = {
  activeSessionId: string;
  durableAttention?: AutoRouteAttention | null;
  attentionStatus: string;
  cancelledContent: string;
  choosingStatus: string;
  setSendingForSession: (sessionId: string, value: boolean) => void;
  setProcessingForSession: (sessionId: string, value: boolean) => void;
  setStatusForSession: (sessionId: string, value: string) => void;
  onOpenModels?: () => void;
  onResumePersistedTurn: (
    attention: AutoRouteAttention,
    choice: Exclude<AutoRouteTurnChoice, "cancel">,
  ) => Promise<boolean>;
  restoreEnabled: boolean;
  recoverableTurnIdentityKeys?: ReadonlySet<string>;
  terminalTurnIds?: ReadonlySet<string>;
};

function createAttention(
  context: ChatTurnContext,
  localProviderId: string,
  localModelId: string,
  cloudModelId: string,
  error: unknown,
  recommendedLocalProviderId: string,
  recommendedLocalModelId: string,
): AutoRouteAttention {
  const evidence = autoRouteRecoveryEvidence(error);
  return {
    sessionId: context.sessionId,
    rootTurnId: context.ancestry.rootTurnId,
    turnId: context.turnId,
    generationToken: context.generationToken,
    localProviderId,
    localModelId,
    recommendedLocalProviderId,
    recommendedLocalModelId,
    cloudModelId,
    failureCode: evidence.code,
    failureBoundary: evidence.boundary,
    kind: evidence.kind,
    continueWhenReady: false,
  };
}

async function resolveTurnChoice(
  attention: AutoRouteAttention,
  choice: AutoRouteRecoveryAction,
  onOpenModels?: () => void,
): Promise<AutoRouteTurnChoice | null> {
  if (choice === "continue_when_ready") return null;
  if (choice === "open_models") {
    if (!onOpenModels) throw new Error("auto_route_models_unavailable");
    onOpenModels();
    return null;
  }
  if (choice === "repair_model") {
    const localProviderId = attention.recommendedLocalProviderId || attention.localProviderId;
    const localModelId = attention.recommendedLocalModelId || attention.localModelId;
    if (!localProviderId || !localModelId) throw new Error("auto_route_repair_model_missing");
    await invoke("repair_auto_route_session_baseline", {
      request: {
        sessionId: attention.sessionId,
        turnId: attention.turnId,
        generationToken: attention.generationToken,
        localProviderId,
        localModelId,
      },
    });
    return "retry";
  }
  if (choice !== "check_saved_work") return choice;
  const readiness = normalizeAutoRouteSessionReadiness(
    await invoke("get_auto_route_session_readiness", { sessionId: attention.sessionId }),
    attention.sessionId,
  );
  if (readiness.status !== "ready") {
    throw new Error(readiness.failureCode ?? "auto_route_saved_work_not_ready");
  }
  return "retry";
}

function persistAttention(attention: AutoRouteAttention) {
  return persistTurnRecovery({
    type: "auto_route",
    sessionId: attention.sessionId,
    rootTurnId: attention.rootTurnId,
    turnId: attention.turnId,
    generationToken: attention.generationToken,
    attention,
    updatedAtMs: Date.now(),
  });
}

function cancelSavedTurn(attention: AutoRouteAttention, content: string) {
  return invoke("cancel_saved_chat_turn", {
    request: {
      sessionId: attention.sessionId,
      turnId: attention.turnId,
      generationToken: attention.generationToken,
      content,
    },
  });
}

function useRestoreAutoRouteAttention(options: {
  activeSessionId: string;
  attention: AutoRouteAttention | null;
  durableAttention: AutoRouteAttention | null;
  attentionStatus: string;
  restoreEnabled: boolean;
  resumedRef: MutableRefObject<Set<string>>;
  resolversRef: MutableRefObject<Map<string, (choice: AutoRouteTurnChoice) => void>>;
  setAttentionForSession: (sessionId: string, value: AutoRouteAttention | null) => void;
  setProcessingForSession: (sessionId: string, value: boolean) => void;
  setSendingForSession: (sessionId: string, value: boolean) => void;
  setStatusForSession: (sessionId: string, value: string) => void;
  terminalTurnIds: ReadonlySet<string>;
  recoverableTurnIdentityKeys: ReadonlySet<string>;
}) {
  useEffect(() => {
    if (!options.restoreEnabled || !options.activeSessionId || options.attention) return;
    const persisted = options.durableAttention ? null : readMatchingTurnRecovery(
      options.activeSessionId, "auto_route", options.recoverableTurnIdentityKeys,
    );
    const candidate = options.durableAttention ?? persisted?.attention ?? null;
    if (!candidate || candidate.sessionId !== options.activeSessionId) return;
    if (options.terminalTurnIds.has(candidate.turnId)) {
      clearTurnRecovery(candidate, "auto_route");
      return;
    }
    if (options.resumedRef.current.has(recoveryKey(candidate))) return;
    if (options.durableAttention) persistAttention(options.durableAttention);
    options.setAttentionForSession(options.activeSessionId, candidate);
    options.setSendingForSession(options.activeSessionId, false);
    options.setProcessingForSession(options.activeSessionId, false);
    options.setStatusForSession(options.activeSessionId, options.attentionStatus);
  }, [options]);
  useEffect(() => () => options.resolversRef.current.clear(), [options.resolversRef]);
}

export function useAutoRouteTurnChoice({
  activeSessionId,
  durableAttention = null,
  attentionStatus,
  cancelledContent,
  choosingStatus,
  setSendingForSession,
  setProcessingForSession,
  setStatusForSession,
  onOpenModels,
  onResumePersistedTurn,
  restoreEnabled,
  recoverableTurnIdentityKeys = noRecoverableTurnIdentities,
  terminalTurnIds = noTerminalTurns,
}: UseAutoRouteTurnChoiceOptions) {
  const [attention, , setAttentionForSession, clearSessionAttention] =
    useSessionScopedState<AutoRouteAttention | null>(activeSessionId, null);
  const resolversRef = useRef(new Map<string, (choice: AutoRouteTurnChoice) => void>());
  const resumedRef = useRef(new Set<string>());

  const requestChoice = useStableEvent((
    context: ChatTurnContext,
    localProviderId: string,
    localModelId: string,
    cloudModelId: string,
    error: unknown,
    recommendedLocalProviderId: string = "",
    recommendedLocalModelId: string = "",
  ) => {
    const next = createAttention(
      context, localProviderId, localModelId, cloudModelId, error,
      recommendedLocalProviderId, recommendedLocalModelId,
    );
    if (!persistAttention(next)) return Promise.reject(new Error("chat_turn_persistence_failed"));
    return new Promise<AutoRouteTurnChoice>((resolve) => {
        resolversRef.current.get(context.sessionId)?.("cancel");
        resolversRef.current.set(context.sessionId, resolve);
        setSendingForSession(context.sessionId, false);
        setProcessingForSession(context.sessionId, false);
        setStatusForSession(context.sessionId, attentionStatus);
        setAttentionForSession(context.sessionId, next);
    });
  });

  const resolveChoice = useStableEvent(async (choice: AutoRouteRecoveryAction) => {
    if (!attention) {
      return;
    }
    const { sessionId } = attention;
    if (choice === "continue_when_ready") {
      const armed = { ...attention, continueWhenReady: true };
      if (!persistAttention(armed)) throw new Error("chat_turn_persistence_failed");
      setAttentionForSession(sessionId, armed);
      return;
    }
    const turnChoice = await resolveTurnChoice(attention, choice, onOpenModels);
    if (!turnChoice) return;
    if (turnChoice === "cancel") {
      await cancelSavedTurn(attention, cancelledContent);
      clearTurnRecovery(attention, "auto_route");
    }
    const resolve = resolversRef.current.get(sessionId);
    const resumedPersistedTurn = !resolve && turnChoice !== "cancel";
    if (resumedPersistedTurn) {
      const resumed = await onResumePersistedTurn(attention, turnChoice);
      if (!resumed) throw new Error("auto_route_saved_turn_unavailable");
    }
    resolversRef.current.delete(sessionId);
    resumedRef.current.add(recoveryKey(attention));
    clearSessionAttention(sessionId);
    if (turnChoice !== "cancel" && !resumedPersistedTurn) {
      setSendingForSession(sessionId, true);
      setProcessingForSession(sessionId, true);
      setStatusForSession(sessionId, choosingStatus);
    }
    resolve?.(turnChoice);
  });

  const cancelChoiceForSession = useStableEvent(async (sessionId: string) => {
    const current = attention?.sessionId === sessionId
      ? attention
      : readMatchingTurnRecovery(sessionId, "auto_route", recoverableTurnIdentityKeys)?.attention
        ?? null;
    if (current) {
      await cancelSavedTurn(current, cancelledContent);
    }
    resolversRef.current.get(sessionId)?.("cancel");
    resolversRef.current.delete(sessionId);
    if (current) clearTurnRecovery(current, "auto_route");
    if (current) resumedRef.current.delete(recoveryKey(current));
    clearSessionAttention(sessionId);
  });

  useRestoreAutoRouteAttention({ activeSessionId, attention, durableAttention, attentionStatus, restoreEnabled,
    resumedRef, resolversRef, setAttentionForSession, setProcessingForSession,
    setSendingForSession, setStatusForSession, terminalTurnIds, recoverableTurnIdentityKeys });

  return {
    autoRouteAttention: attention,
    requestAutoRouteTurnChoice: requestChoice,
    resolveAutoRouteTurnChoice: resolveChoice,
    cancelAutoRouteTurnChoiceForSession: cancelChoiceForSession,
  };
}
