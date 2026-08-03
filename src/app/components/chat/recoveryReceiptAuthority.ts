import type { AgentExecutionRecoveryState } from "./agentExecutionRecovery";

export type RecoveryReceiptAuthority =
  | "checking"
  | "current"
  | "inactive"
  | "unavailable";

type RecoverableReceipt = {
  executionId: string;
  planId: string;
  recoverable: boolean;
  recoveryAction: string;
  code: string;
  context: {
    nextOperation: string | null;
    frozenArgumentSha256: string | null;
  };
};

type RecoveryReceiptAuthorityEntry = {
  messageId: number;
  receipt: RecoverableReceipt;
};

type RecoveryProjectionStatus = "idle" | "loading" | "ready" | "failed";

const sha256Pattern = /^[a-f0-9]{64}$/;

function executionPlanKey(executionId: string, planId: string) {
  return JSON.stringify([executionId, planId]);
}

function isFrozenMailInterruption(receipt: RecoverableReceipt) {
  return receipt.code === "agent_execution_interrupted"
    && receipt.recoveryAction === "resume_same_execution"
    && receipt.context.nextOperation === "draft_release_recovery_email"
    && receipt.context.frozenArgumentSha256 !== null
    && sha256Pattern.test(receipt.context.frozenArgumentSha256);
}

/**
 * Projects durable execution state onto individual transcript receipts.
 * Only the latest recoverable receipt for an exact execution and plan can be
 * current; every historical receipt is intentionally inert.
 */
export function recoveryReceiptAuthoritiesForTranscript(
  entries: readonly RecoveryReceiptAuthorityEntry[],
  projectionStatus: RecoveryProjectionStatus,
  states: ReadonlyMap<string, AgentExecutionRecoveryState>,
) {
  const authorities = new Map<number, RecoveryReceiptAuthority>();
  const recoverableEntries = entries.filter(({ receipt }) => receipt.recoverable);

  if (projectionStatus === "idle" || projectionStatus === "loading") {
    for (const { messageId } of recoverableEntries) authorities.set(messageId, "checking");
    return authorities;
  }
  if (projectionStatus === "failed") {
    for (const { messageId } of recoverableEntries) authorities.set(messageId, "unavailable");
    return authorities;
  }

  const latestMessageIdByExecutionPlan = new Map<string, number>();
  for (const { messageId, receipt } of recoverableEntries) {
    latestMessageIdByExecutionPlan.set(
      executionPlanKey(receipt.executionId, receipt.planId),
      messageId,
    );
  }

  for (const { messageId, receipt } of recoverableEntries) {
    const state = states.get(receipt.executionId);
    const isLatest = latestMessageIdByExecutionPlan.get(
      executionPlanKey(receipt.executionId, receipt.planId),
    ) === messageId;
    const exactHaltedExecution = state?.executionId === receipt.executionId
      && state.planId === receipt.planId
      && state.status === "halted";
    const phaseAllowsReceipt = state?.terminalPhase !== "restart_recovery_ready"
      || isFrozenMailInterruption(receipt);

    authorities.set(
      messageId,
      isLatest && exactHaltedExecution && phaseAllowsReceipt ? "current" : "inactive",
    );
  }
  return authorities;
}

export function terminalRecoveryProjectionRefreshKey(input: {
  sessionId: string;
  executionId: string;
  status: "running" | "completed" | "failed" | "halted" | null;
}) {
  if (!input.sessionId || !input.executionId || !input.status || input.status === "running") {
    return null;
  }
  return JSON.stringify([input.sessionId, input.executionId, input.status]);
}
