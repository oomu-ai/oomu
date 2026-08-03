import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { isTauriRuntime } from "@/lib/invoke";
import { parseAgentExecutionRecoveryReceipt } from "./RecoveryReceiptCard";
import {
  agentRecoveryActionKey,
  getAgentExecutionRecoveryStates,
  type AgentExecutionRecoveryState,
} from "./agentExecutionRecovery";
import {
  shouldSynthesizeResumeActionKey,
  type ActiveAgentExecution,
} from "./agentExecutionState";
import {
  recoveryReceiptAuthoritiesForTranscript,
  terminalRecoveryProjectionRefreshKey,
} from "./recoveryReceiptAuthority";

type RecoveryProjectionMessage = {
  id: number;
  role: "user" | "assistant" | "system";
  content: string;
  metadata?: {
    turnId?: string;
    rootTurnId?: string;
    generationToken?: string;
    turnState?: string;
  } | null;
};

function recoveryTurnKey(value: {
  sessionId: string;
  rootTurnId: string;
  failedTurnId: string;
  generationToken: string;
}) {
  return JSON.stringify([
    value.sessionId,
    value.rootTurnId,
    value.failedTurnId,
    value.generationToken,
  ]);
}

function acceptedRecoveryTurnKey(
  message: RecoveryProjectionMessage,
  sessionId: string,
) {
  const failedTurnId = message.metadata?.turnId;
  const generationToken = message.metadata?.generationToken;
  if (
    message.role !== "user"
    || !failedTurnId
    || !generationToken
    || !["accepted", "interrupted", "escalated"].includes(message.metadata?.turnState ?? "")
  ) return null;
  return recoveryTurnKey({
    sessionId,
    rootTurnId: message.metadata?.rootTurnId ?? failedTurnId,
    failedTurnId,
    generationToken,
  });
}

function acceptedRecoveryTurnKeyText(
  messages: readonly RecoveryProjectionMessage[],
  sessionId: string,
) {
  return messages.map((message) => acceptedRecoveryTurnKey(message, sessionId))
    .filter((key): key is string => Boolean(key)).sort().join("\n");
}

function exactRecoveryStates(
  states: AgentExecutionRecoveryState[],
  acceptedRecoveryTurnKeys: ReadonlySet<string>,
) {
  return new Map(states.filter((state) => state.sessionId
    && state.rootTurnId
    && state.failedTurnId
    && state.generationToken
    && acceptedRecoveryTurnKeys.has(recoveryTurnKey({
      sessionId: state.sessionId,
      rootTurnId: state.rootTurnId,
      failedTurnId: state.failedTurnId,
      generationToken: state.generationToken,
    }))).map((state) => [state.executionId, state]));
}

export type RecoveryExecutionStateSnapshot = {
  sessionId: string;
  status: "idle" | "loading" | "ready" | "failed";
  byExecutionId: ReadonlyMap<string, AgentExecutionRecoveryState>;
};

export function useRecoveryReceiptProjection(input: {
  activeExecution: ActiveAgentExecution | null;
  activeSessionId: string;
  completedRecoveryActionKeys: ReadonlySet<string>;
  messages: readonly RecoveryProjectionMessage[];
}) {
  const { activeExecution, activeSessionId, completedRecoveryActionKeys, messages } = input;
  const activeSessionIdRef = useRef(activeSessionId);
  const lastTerminalRefreshKeyRef = useRef("");
  const [refreshNonce, setRefreshNonce] = useState(0);
  const [settledSnapshot, setSettledSnapshot] = useState<RecoveryExecutionStateSnapshot & {
    requestKey: string;
  }>({
    requestKey: "",
    sessionId: "",
    status: "idle",
    byExecutionId: new Map(),
  });
  useLayoutEffect(() => {
    activeSessionIdRef.current = activeSessionId;
  }, [activeSessionId]);
  const receiptEntries = useMemo(() => messages.flatMap((message) => {
    if (message.role !== "assistant") return [];
    const receipt = parseAgentExecutionRecoveryReceipt(message.content);
    return receipt ? [{ messageId: message.id, receipt }] : [];
  }), [messages]);
  const receipts = useMemo(
    () => receiptEntries.map(({ receipt }) => receipt),
    [receiptEntries],
  );
  const sessionId = activeSessionId.trim();
  const acceptedRecoveryTurnKeysText = useMemo(
    () => acceptedRecoveryTurnKeyText(messages, sessionId),
    [messages, sessionId],
  );
  const acceptedRecoveryTurnKeys = useMemo(
    () => new Set(acceptedRecoveryTurnKeysText ? acceptedRecoveryTurnKeysText.split("\n") : []),
    [acceptedRecoveryTurnKeysText],
  );
  const executionIdsKey = useMemo(
    () => Array.from(new Set(receipts.map(({ executionId }) => executionId))).sort().join("\n"),
    [receipts],
  );
  const executionIds = useMemo(
    () => executionIdsKey ? executionIdsKey.split("\n") : [],
    [executionIdsKey],
  );
  const queryEnabled = Boolean(sessionId && isTauriRuntime && executionIds.length > 0);
  const requestKey = useMemo(
    () => JSON.stringify([sessionId, executionIdsKey, refreshNonce]),
    [executionIdsKey, refreshNonce, sessionId],
  );
  useEffect(() => {
    if (!queryEnabled) return;
    let cancelled = false;
    void getAgentExecutionRecoveryStates(sessionId, executionIds)
      .then((states) => {
        if (cancelled || activeSessionIdRef.current !== sessionId) return;
        setSettledSnapshot({
          requestKey,
          sessionId,
          status: "ready",
          byExecutionId: exactRecoveryStates(states, acceptedRecoveryTurnKeys),
        });
      })
      .catch(() => {
        if (cancelled || activeSessionIdRef.current !== sessionId) return;
        setSettledSnapshot({
          requestKey,
          sessionId,
          status: "failed",
          byExecutionId: new Map(),
        });
      });
    return () => { cancelled = true; };
  }, [acceptedRecoveryTurnKeys, executionIds, queryEnabled, requestKey, sessionId]);
  const snapshot = useMemo<RecoveryExecutionStateSnapshot>(() => {
    if (!queryEnabled) return { sessionId, status: "ready", byExecutionId: new Map() };
    if (settledSnapshot.requestKey !== requestKey) {
      return { sessionId, status: "loading", byExecutionId: new Map() };
    }
    return settledSnapshot;
  }, [queryEnabled, requestKey, sessionId, settledSnapshot]);
  const refresh = useCallback(() => setRefreshNonce((current) => current + 1), []);
  const refreshForTerminalBatch = useCallback((
    sessionId: string,
    executionId: string,
    status: ActiveAgentExecution["status"],
  ) => {
    if (
      activeSessionIdRef.current === sessionId
      && terminalRecoveryProjectionRefreshKey({ sessionId, executionId, status })
    ) {
      refresh();
    }
  }, [refresh]);
  useEffect(() => {
    const sessionId = activeExecution?.sessionId ?? "";
    const executionId = activeExecution?.executionId ?? "";
    if (sessionId !== activeSessionIdRef.current) return;
    const refreshKey = terminalRecoveryProjectionRefreshKey({
      sessionId,
      executionId,
      status: activeExecution?.status ?? null,
    });
    if (!refreshKey || lastTerminalRefreshKeyRef.current === refreshKey) return;
    lastTerminalRefreshKeyRef.current = refreshKey;
    refresh();
  }, [activeExecution?.executionId, activeExecution?.sessionId, activeExecution?.status, refresh]);
  const receiptAuthorities = useMemo(() => recoveryReceiptAuthoritiesForTranscript(
    receiptEntries,
    snapshot.sessionId === activeSessionId ? snapshot.status : "loading",
    snapshot.byExecutionId,
  ), [activeSessionId, receiptEntries, snapshot]);
  const effectiveActionKeys = useMemo(() => {
    let interruptedApprovalIdentity: { executionId: string; planId: string } | null = null;
    if (activeExecution) {
      for (let index = receipts.length - 1; index >= 0; index -= 1) {
        const receipt = receipts[index];
        if (
          receipt.executionId === activeExecution.executionId
          && receipt.planId === activeExecution.planId
          && receipt.code === "agent_execution_interrupted"
          && receipt.recoveryAction === "resume_same_execution"
          && receipt.context.nextOperation === "draft_release_recovery_email"
          && receipt.context.frozenArgumentSha256 !== null
        ) {
          interruptedApprovalIdentity = {
            executionId: receipt.executionId,
            planId: receipt.planId,
          };
          break;
        }
      }
    }
    const durableKeys = new Set(completedRecoveryActionKeys);
    if (snapshot.sessionId === activeSessionId && snapshot.status === "ready") {
      for (const receipt of receipts) {
        if (receipt.recoveryAction !== "resume_same_execution") continue;
        const state = snapshot.byExecutionId.get(receipt.executionId);
        if (state?.planId === receipt.planId && state.status !== "halted") {
          durableKeys.add(agentRecoveryActionKey(receipt.executionId, "resume_same_execution"));
        }
      }
    }
    return shouldSynthesizeResumeActionKey(activeExecution, interruptedApprovalIdentity)
      ? durableKeys.add(agentRecoveryActionKey(activeExecution!.executionId, "resume_same_execution"))
      : durableKeys;
  }, [activeExecution, activeSessionId, completedRecoveryActionKeys, receipts, snapshot]);
  return {
    effectiveActionKeys,
    receiptAuthorities,
    refresh,
    refreshForTerminalBatch,
    snapshot,
  };
}
