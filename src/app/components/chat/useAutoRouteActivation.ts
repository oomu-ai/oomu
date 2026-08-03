import { invoke } from "@/lib/invoke";
import { useEffect, useRef, useState } from "react";

import type { ChatSession } from "@/lib/chatSessions";
import type {
  CanonicalModelId,
  ProviderConfigurationId,
  ProviderTypeId,
} from "@/lib/modelRegistry";

import type { AutoRouteActivationFailure } from "./AutoRouteActivationRecoveryCard";
import { stableErrorCode } from "./inferenceErrors";
import type { RouteOverride } from "./sessionRouting";

export type AutoRouteBaselineIpc = {
  providerConfigId: ProviderConfigurationId;
  providerType: ProviderTypeId;
  modelId: CanonicalModelId;
  reasoningDepth: string;
  contextBudget: number;
};

type AutoRouteActivationResponse = {
  session: ChatSession;
  receipt: {
    kind: string;
    receiptId: string;
    dynamicRoutingEnabled: boolean;
    committed: boolean;
    rolledBack: boolean;
    changed: boolean;
    errorCode?: string;
  };
};

export type PendingAutoRouteActivationFailure = AutoRouteActivationFailure;

const RETRYABLE_CODES = new Set([
  "auto_route_activation_persistence_failed",
  "auto_route_activation_worker_failed",
  "auto_route_local_model_store_unavailable",
  "auto_route_provider_store_unavailable",
]);

type SessionMutation = { sessionId: string; hydrationLockToken: number | null };
type AutoRouteActivationOptions = {
  activeSessionId: string;
  buildBaseline: (route: RouteOverride) => AutoRouteBaselineIpc;
  canActivate: boolean;
  dynamicRoutingEnabled: boolean;
  ensureSession: (preferredSessionId?: string | null) => Promise<SessionMutation>;
  getRoute: () => RouteOverride;
  onSessionsChange: (sessions: ChatSession[]) => void;
  sessions: ChatSession[];
  setStatus: (status: string) => void;
  statusBlocked: string;
  statusDisabled: string;
  statusEnabled: string;
  unlockSession: (sessionId: string, token: number) => void;
};
type ActivationFailureState = {
  ownerSessionId: string;
  failure: PendingAutoRouteActivationFailure | null;
};

function toActivationFailure(
  error: unknown,
  sessionId: string,
  desiredEnabled: boolean,
): PendingAutoRouteActivationFailure {
  const code = stableErrorCode(error) || "auto_route_activation_unknown";
  return { sessionId, code, retryable: RETRYABLE_CODES.has(code), desiredEnabled };
}

export function useAutoRouteActivation({
  activeSessionId,
  buildBaseline,
  canActivate,
  dynamicRoutingEnabled,
  ensureSession,
  getRoute,
  onSessionsChange,
  sessions,
  setStatus,
  statusBlocked,
  statusDisabled,
  statusEnabled,
  unlockSession,
}: AutoRouteActivationOptions) {
  const [failureState, setFailureState] = useState<ActivationFailureState>(
    { ownerSessionId: activeSessionId, failure: null });
  const [isSaving, setIsSaving] = useState(false);
  const [refreshNonce, setRefreshNonce] = useState(0);
  const mutationInFlight = useRef(false);
  const pendingActivation = useRef<{ sessionId: string; desiredEnabled: boolean } | null>(null);
  const sessionsRef = useRef(sessions);

  if (failureState.ownerSessionId !== activeSessionId) {
    setFailureState({ ownerSessionId: activeSessionId, failure: null });
  }

  useEffect(() => {
    sessionsRef.current = sessions;
  }, [sessions]);

  useEffect(() => {
    if (
      pendingActivation.current?.sessionId
      && pendingActivation.current.sessionId !== activeSessionId
    ) {
      pendingActivation.current = null;
    }
  }, [activeSessionId]);

  async function commit(enabled: boolean, requestedSessionId: string) {
    let mutation: SessionMutation = { sessionId: "", hydrationLockToken: null };
    mutationInFlight.current = true;
    setIsSaving(true);
    try {
      mutation = await ensureSession(requestedSessionId || null);
      if (!mutation.sessionId) throw new Error("auto_route_session_required");
      const response = await invoke<AutoRouteActivationResponse>(
        "update_chat_session_dynamic_routing_override",
        {
          sessionId: mutation.sessionId,
          dynamicRoutingOverride: enabled,
          autoRouteBaseline: enabled ? buildBaseline(getRoute()) : null,
        },
      );
      if (
        !response.receipt.committed
        || response.receipt.rolledBack
        || response.receipt.dynamicRoutingEnabled !== enabled
      ) {
        throw new Error(response.receipt.errorCode ?? "auto_route_activation_not_committed");
      }
      onSessionsChange([
        response.session,
        ...sessionsRef.current.filter((session) => session.id !== response.session.id),
      ]);
      setFailureState((current) =>
        current.ownerSessionId === mutation.sessionId
          ? { ...current, failure: null }
          : current
      );
      setStatus(enabled ? statusEnabled : statusDisabled);
    } catch (error) {
      const nextFailure = toActivationFailure(
        error,
        mutation.sessionId || activeSessionId,
        enabled,
      );
      setFailureState((current) =>
        current.ownerSessionId !== nextFailure.sessionId
          || current.failure?.code === nextFailure.code
          && current.failure.desiredEnabled === nextFailure.desiredEnabled
          ? current
          : { ...current, failure: nextFailure }
      );
      setStatus(statusBlocked);
    } finally {
      if (mutation.sessionId && mutation.hydrationLockToken !== null) {
        unlockSession(mutation.sessionId, mutation.hydrationLockToken);
      }
      setIsSaving(false);
      mutationInFlight.current = false;
      const pending = pendingActivation.current;
      if (pending?.sessionId === requestedSessionId) {
        pendingActivation.current = null;
        const replaySessionId = pending.sessionId || mutation.sessionId;
        queueMicrotask(() => void commit(pending.desiredEnabled, replaySessionId));
      }
    }
  }

  async function toggle(desiredEnabled?: boolean) {
    if (!canActivate) return;
    const enabled = desiredEnabled ?? !dynamicRoutingEnabled;
    if (mutationInFlight.current) {
      pendingActivation.current = { sessionId: activeSessionId, desiredEnabled: enabled };
      return;
    }
    return commit(enabled, activeSessionId);
  }

  function keepCommittedRoute() {
    setFailureState((current) =>
      current.ownerSessionId === activeSessionId
        ? { ...current, failure: null } : current);
    setRefreshNonce((current) => current + 1);
  }
  const failure = failureState.ownerSessionId === activeSessionId ? failureState.failure : null;
  return { failure, isSaving, keepCommittedRoute, refreshNonce, toggle };
}
