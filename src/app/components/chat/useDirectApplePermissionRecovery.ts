import { useEffect, useRef } from "react";
import { invoke } from "@/lib/invoke";
import type { ChatTurnContext } from "@/lib/chatTurnContext";
import { localToolFailureCode } from "./mcpToolResults";
import {
  macPermissionRecoveryDescriptor,
  type MacPermissionRecoveryDescriptor,
} from "./MacPermissionRecoveryCard";
import { stableErrorCode } from "./inferenceErrors";
import { openMacPermissionSettings } from "./agentExecutionRecovery";
import { useSessionScopedState, useStableEvent } from "./sessionScopedState";
import {
  clearTurnRecovery,
  persistTurnRecovery,
  readMatchingTurnRecovery,
  readTurnRecovery,
} from "./turnRecoveryPersistence";

const noTerminalTurns = new Set<string>();
const noRecoverableTurnIdentities = new Set<string>();
const recoveryKey = (value: { turnId: string; generationToken: string }) =>
  `${value.turnId}:${value.generationToken}`;

export type DirectApplePermissionAttention = {
  sessionId: string;
  rootTurnId: string;
  turnId: string;
  generationToken: string;
  boundary: string;
  code: string;
  descriptor: MacPermissionRecoveryDescriptor;
};

export type DirectApplePermissionChoice = "retry" | "cancel";

type Options = {
  activeSessionId: string;
  attentionStatus: string;
  choosingStatus: string;
  durableAttention?: DirectApplePermissionAttention | null;
  onResumePersistedTurn: (attention: DirectApplePermissionAttention) => Promise<boolean>;
  restoreEnabled: boolean;
  recoverableTurnIdentityKeys?: ReadonlySet<string>;
  terminalTurnIds?: ReadonlySet<string>;
  setProcessingForSession: (sessionId: string, value: boolean) => void;
  setSendingForSession: (sessionId: string, value: boolean) => void;
  setStatusForSession: (sessionId: string, value: string) => void;
};

function recoverablePermission(error: unknown, capabilityId: string) {
  const rawCode = stableErrorCode(error) || localToolFailureCode(error);
  const code = rawCode === "timeout" ? `${capabilityId}_permission_timeout` : rawCode;
  const descriptor = macPermissionRecoveryDescriptor(code, capabilityId);
  return descriptor ? { code, descriptor } : null;
}

async function cancelPersistedPermissionTurn(attention: DirectApplePermissionAttention) {
  await invoke("cancel_permission_recovery_turn", { request: {
    sessionId: attention.sessionId,
    turnId: attention.turnId,
    generationToken: attention.generationToken,
    capabilityId: attention.descriptor.capabilityId,
  } });
}

function clearTerminalPermissionRecovery(
  sessionId: string,
  recoverableTurnIdentityKeys: ReadonlySet<string>,
  terminalTurnIds: ReadonlySet<string>,
) {
  const cached = readMatchingTurnRecovery(
    sessionId,
    "apple_permission",
    recoverableTurnIdentityKeys,
  ) ?? readTurnRecovery(sessionId, "apple_permission");
  if (cached && terminalTurnIds.has(cached.turnId)) {
    clearTurnRecovery(cached, "apple_permission");
  }
}

export function useDirectApplePermissionRecovery({
  activeSessionId,
  attentionStatus,
  choosingStatus,
  durableAttention = null,
  onResumePersistedTurn,
  restoreEnabled,
  recoverableTurnIdentityKeys = noRecoverableTurnIdentities,
  terminalTurnIds = noTerminalTurns,
  setProcessingForSession,
  setSendingForSession,
  setStatusForSession,
}: Options) {
  const [attention, , setAttentionForSession, clearSessionAttention] =
    useSessionScopedState<DirectApplePermissionAttention | null>(activeSessionId, null);
  const resolversRef = useRef(new Map<string, (choice: DirectApplePermissionChoice) => void>());
  const resumedRef = useRef(new Set<string>());
  const requestRecovery = useStableEvent((
    context: ChatTurnContext,
    capabilityId: string,
    error: unknown,
  ) => {
    const evidence = recoverablePermission(error, capabilityId);
    if (!evidence) return null;
    const next: DirectApplePermissionAttention = {
      sessionId: context.sessionId,
      rootTurnId: context.ancestry.rootTurnId,
      turnId: context.turnId,
      generationToken: context.generationToken,
      boundary: "direct_apple_read",
      code: evidence.code,
      descriptor: evidence.descriptor,
    };
    persistTurnRecovery({
      type: "apple_permission",
      ...next,
      updatedAtMs: Date.now(),
    });
    return new Promise<DirectApplePermissionChoice>((resolve) => {
      resolversRef.current.get(context.sessionId)?.("cancel");
      resolversRef.current.set(context.sessionId, resolve);
      setSendingForSession(context.sessionId, false);
      setProcessingForSession(context.sessionId, false);
      setStatusForSession(context.sessionId, attentionStatus);
      setAttentionForSession(context.sessionId, next);
    });
  });
  const resolveRecovery = useStableEvent(async (choice: DirectApplePermissionChoice) => {
    if (!attention) return;
    const resolve = resolversRef.current.get(attention.sessionId);
    if (!resolve && choice === "retry") {
      const resumed = await onResumePersistedTurn(attention);
      if (!resumed) throw new Error("apple_permission_saved_turn_unavailable");
    }
    if (!resolve && choice === "cancel") {
      await cancelPersistedPermissionTurn(attention);
    }
    resolversRef.current.delete(attention.sessionId);
    resumedRef.current.add(recoveryKey(attention));
    clearSessionAttention(attention.sessionId);
    if (choice === "retry") {
      setSendingForSession(attention.sessionId, true);
      setProcessingForSession(attention.sessionId, true);
      setStatusForSession(attention.sessionId, choosingStatus);
    }
    resolve?.(choice);
  });
  const openSettings = useStableEvent(async (_recoveryId: string, capabilityId: string) => {
    await openMacPermissionSettings(capabilityId);
  });
  const checkAgain = useStableEvent(async () => resolveRecovery("retry"));
  const cancel = useStableEvent(async () => resolveRecovery("cancel"));
  useEffect(() => {
    if (!restoreEnabled || !activeSessionId || attention) return;
    const persisted = durableAttention;
    if (!persisted) return;
    if (terminalTurnIds.has(persisted.turnId)) {
      clearTurnRecovery(persisted, "apple_permission");
      return;
    }
    if (resumedRef.current.has(recoveryKey(persisted))) return;
    const restored: DirectApplePermissionAttention = {
      sessionId: persisted.sessionId,
      rootTurnId: persisted.rootTurnId,
      turnId: persisted.turnId,
      generationToken: persisted.generationToken,
      boundary: persisted.boundary,
      code: persisted.code,
      descriptor: persisted.descriptor,
    };
    setAttentionForSession(activeSessionId, restored);
    setSendingForSession(activeSessionId, false);
    setProcessingForSession(activeSessionId, false);
    setStatusForSession(activeSessionId, attentionStatus);
  }, [activeSessionId, attention, attentionStatus, durableAttention, restoreEnabled, terminalTurnIds,
    setAttentionForSession,
    setProcessingForSession, setSendingForSession, setStatusForSession]);

  useEffect(() => {
    clearTerminalPermissionRecovery(
      activeSessionId,
      recoverableTurnIdentityKeys,
      terminalTurnIds,
    );
  }, [activeSessionId, recoverableTurnIdentityKeys, terminalTurnIds]);

  useEffect(() => () => {
    resolversRef.current.clear();
  }, []);

  return {
    directApplePermissionAttention: attention,
    requestDirectApplePermissionRecovery: requestRecovery,
    directApplePermissionActions: {
      onCancel: cancel,
      onCheck: checkAgain,
      onOpenSettings: openSettings,
    },
  };
}
