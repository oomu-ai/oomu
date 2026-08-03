import { useEffect, useMemo } from "react";
import type { AutoRouteAttention } from "./AutoRouteAttentionCard";
import { useAutoRouteTurnChoice } from "./useAutoRouteTurnChoice";
import { useDirectApplePermissionRecovery } from "./useDirectApplePermissionRecovery";
import { macPermissionRecoveryDescriptor } from "./MacPermissionRecoveryCard";
import {
  usePersistedTurnReplay,
  type PersistedTurnReplayMessage,
  type PersistedTurnReplaySubmitOptions,
} from "./usePersistedTurnReplay";
import {
  clearTerminalTurnRecoveries,
  turnRecoveryIdentityKey,
} from "./turnRecoveryPersistence";

export type { PersistedTurnReplaySubmitOptions } from "./usePersistedTurnReplay";

type SessionBooleanSetter = (sessionId: string, value: boolean) => void;
type SessionStringSetter = (sessionId: string, value: string) => void;

type Options = {
  activeSessionId: string;
  messages: readonly PersistedTurnReplayMessage[];
  onOpenModels?: () => void;
  restoreEnabled: boolean;
  setProcessingForSession: SessionBooleanSetter;
  setSendingForSession: SessionBooleanSetter;
  setStatusForSession: SessionStringSetter;
  submit: (message: string, options: PersistedTurnReplaySubmitOptions) => Promise<void>;
  translate: (key: string) => string;
};

export function interruptedTurnAttentionForSession(
  sessionId: string,
  messages: readonly PersistedTurnReplayMessage[],
): AutoRouteAttention | null {
  const message = [...messages].reverse().find((candidate) =>
    candidate.role === "user"
    && candidate.metadata?.turnState === "interrupted"
    && candidate.metadata.permissionContinuation?.state !== "waiting"
  );
  const turnId = message?.metadata?.turnId?.trim() ?? "";
  const generationToken = message?.metadata?.generationToken?.trim() ?? "";
  if (!sessionId.trim() || !turnId || !generationToken) return null;
  return {
    sessionId,
    rootTurnId: message?.metadata?.rootTurnId?.trim() || turnId,
    turnId,
    generationToken,
    localProviderId: message?.providerId?.trim() ?? "",
    localModelId: message?.modelId?.trim() ?? "",
    recommendedLocalProviderId: "",
    recommendedLocalModelId: "",
    cloudModelId: "",
    failureCode: "turn_interrupted",
    failureBoundary: "user_stop",
    kind: "interrupted",
    continueWhenReady: false,
  };
}

export function useChatTurnRecovery(options: Options) {
  const durableInterruptedAttention = useMemo(
    () => interruptedTurnAttentionForSession(options.activeSessionId, options.messages),
    [options.activeSessionId, options.messages],
  );
  const recoverableTurnIdentityKeys = useMemo(() => new Set(options.messages.flatMap((message) => {
    const turnId = message.metadata?.turnId;
    const generationToken = message.metadata?.generationToken;
    const turnState = message.metadata?.turnState ?? "";
    if (
      message.role !== "user"
      || !turnId
      || !generationToken
      || !["accepted", "interrupted", "permission_waiting"].includes(turnState)
    ) {
      return [];
    }
    return [turnRecoveryIdentityKey({
      sessionId: options.activeSessionId,
      rootTurnId: message.metadata?.rootTurnId ?? turnId,
      turnId,
      generationToken,
    })];
  })), [options.activeSessionId, options.messages]);
  const terminalTurnIds = useMemo(() => new Set(options.messages.flatMap((message) =>
    message.metadata?.terminalResultForTurnId
      ? [message.metadata.terminalResultForTurnId]
      : []
  )), [options.messages]);
  const terminalTurnIdentityKeys = useMemo(() => new Set(options.messages.flatMap((message) => {
    const turnId = message.metadata?.terminalResultForTurnId;
    const generationToken = message.metadata?.generationToken;
    if (!turnId || !generationToken) return [];
    return [turnRecoveryIdentityKey({
      sessionId: options.activeSessionId,
      rootTurnId: message.metadata?.rootTurnId ?? turnId,
      turnId,
      generationToken,
    })];
  })), [options.activeSessionId, options.messages]);
  useEffect(() => {
    clearTerminalTurnRecoveries(options.activeSessionId, terminalTurnIdentityKeys);
  }, [options.activeSessionId, terminalTurnIdentityKeys]);
  const replay = usePersistedTurnReplay({
    activeSessionId: options.activeSessionId,
    messages: options.messages,
    submit: options.submit,
  });
  const durableApplePermissionAttention = useMemo(() => {
    const message = [...options.messages].reverse().find((candidate) => {
      const continuation = candidate.metadata?.permissionContinuation;
      return candidate.role === "user"
        && ["permission_waiting", "interrupted"].includes(candidate.metadata?.turnState ?? "")
        && continuation?.state === "waiting";
    });
    const continuation = message?.metadata?.permissionContinuation;
    const turnId = message?.metadata?.turnId;
    const generationToken = message?.metadata?.generationToken;
    const errorCode = continuation?.errorCode;
    const descriptor = errorCode && continuation
      ? macPermissionRecoveryDescriptor(errorCode, continuation.capabilityId)
      : null;
    if (!turnId || !generationToken || !errorCode || !descriptor) return null;
    return {
      sessionId: options.activeSessionId,
      rootTurnId: message?.metadata?.rootTurnId ?? turnId,
      turnId,
      generationToken,
      boundary: continuation?.boundary ?? "macos_permission_broker",
      code: errorCode,
      descriptor,
    };
  }, [options.activeSessionId, options.messages]);
  const autoRoute = useAutoRouteTurnChoice({
    activeSessionId: options.activeSessionId,
    durableAttention: durableInterruptedAttention,
    attentionStatus: options.translate("chat.route.needs_attention"),
    cancelledContent: options.translate("tasks.error_cancelled"),
    choosingStatus: options.translate("chat.status.choosing_model"),
    onOpenModels: options.onOpenModels,
    onResumePersistedTurn: replay.resumeAutoRouteTurn,
    restoreEnabled: options.restoreEnabled,
    recoverableTurnIdentityKeys,
    terminalTurnIds,
    setProcessingForSession: options.setProcessingForSession,
    setSendingForSession: options.setSendingForSession,
    setStatusForSession: options.setStatusForSession,
  });
  const applePermission = useDirectApplePermissionRecovery({
    activeSessionId: options.activeSessionId,
    attentionStatus: options.translate("sprint_301.permission_recovery.saved"),
    choosingStatus: options.translate("chat.status.thinking"),
    durableAttention: durableApplePermissionAttention,
    onResumePersistedTurn: replay.resumeApplePermissionTurn,
    restoreEnabled: options.restoreEnabled,
    recoverableTurnIdentityKeys,
    terminalTurnIds,
    setProcessingForSession: options.setProcessingForSession,
    setSendingForSession: options.setSendingForSession,
    setStatusForSession: options.setStatusForSession,
  });
  return { ...autoRoute, ...applePermission };
}
