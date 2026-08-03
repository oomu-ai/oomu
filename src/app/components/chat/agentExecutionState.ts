export type AgentExecutionStartResponse = {
  execution_id?: string;
  executionId?: string;
  plan_id?: string;
  planId?: string;
  session_id?: string;
  sessionId?: string;
  stream_start_after_log_id?: number;
  streamStartAfterLogId?: number;
};

export type AgentPlanAuthorityResponse = {
  authorityProofId?: string | null;
  expiresAtMs?: number | null;
};

type AgentExecutionLogRecord = {
  id: number;
  executionId: string;
  planId: string;
  sessionId?: string | null;
  agentId?: string | null;
  level: "info" | "thought" | "error" | string;
  phase: string;
  message: string;
  payloadJson?: string | null;
  createdAtMs: number;
};

export type AgentExecutionLogBatch = {
  executionId: string;
  logs: AgentExecutionLogRecord[];
  terminal: boolean;
};

export type ActiveAgentExecution = {
  executionId: string;
  planId: string;
  sessionId: string;
  status: "running" | "completed" | "failed" | "halted";
  logs: AgentExecutionLogRecord[];
  lastSeenId: number;
  streamStartAfterLogId?: number;
  startedAtMs: number;
};

const activeExecutionStoragePrefix = "oomu.chat.activeAgentExecution";

function activeExecutionStorageKey(sessionId: string) {
  return `${activeExecutionStoragePrefix}:${sessionId}`;
}

export function executionIdFromStartResponse(response: AgentExecutionStartResponse) {
  return response.execution_id ?? response.executionId ?? "";
}

export function planIdFromStartResponse(response: AgentExecutionStartResponse, fallback: string) {
  return response.plan_id ?? response.planId ?? fallback;
}

export function sessionIdFromStartResponse(response: AgentExecutionStartResponse, fallback: string) {
  return response.session_id ?? response.sessionId ?? fallback;
}

export function streamStartAfterLogIdFromResponse(response: AgentExecutionStartResponse) {
  const value = response.stream_start_after_log_id ?? response.streamStartAfterLogId;
  return Number.isSafeInteger(value) && Number(value) >= 0 ? Number(value) : 0;
}

export function readStoredActiveExecution(sessionId: string): ActiveAgentExecution | null {
  if (typeof window === "undefined" || !sessionId) return null;
  try {
    const stored = window.localStorage.getItem(activeExecutionStorageKey(sessionId));
    if (!stored) return null;
    const parsed = JSON.parse(stored) as Partial<ActiveAgentExecution>;
    if (!parsed.executionId || !parsed.planId || parsed.sessionId !== sessionId) return null;
    const lastSeenId = Number.isSafeInteger(parsed.lastSeenId) && Number(parsed.lastSeenId) >= 0
      ? Number(parsed.lastSeenId)
      : 0;
    return {
      executionId: parsed.executionId,
      planId: parsed.planId,
      sessionId,
      status: parsed.status === "completed" || parsed.status === "failed" || parsed.status === "halted"
        ? parsed.status
        : "running",
      logs: [],
      lastSeenId,
      streamStartAfterLogId: lastSeenId,
      startedAtMs: Number.isFinite(parsed.startedAtMs) ? Number(parsed.startedAtMs) : Date.now(),
    };
  } catch {
    return null;
  }
}

export function persistActiveExecution(execution: ActiveAgentExecution) {
  if (typeof window === "undefined" || !execution.sessionId) return;
  window.localStorage.setItem(activeExecutionStorageKey(execution.sessionId), JSON.stringify({
    executionId: execution.executionId,
    planId: execution.planId,
    sessionId: execution.sessionId,
    status: execution.status,
    lastSeenId: execution.lastSeenId,
    startedAtMs: execution.startedAtMs,
  }));
}

export function clearStoredActiveExecution(sessionId: string) {
  if (typeof window === "undefined" || !sessionId) return;
  window.localStorage.removeItem(activeExecutionStorageKey(sessionId));
}

export function mergeExecutionLogs(
  existing: AgentExecutionLogRecord[],
  incoming: AgentExecutionLogRecord[],
) {
  const byId = new Map<number, AgentExecutionLogRecord>();
  for (const log of existing) byId.set(log.id, log);
  for (const log of incoming) byId.set(log.id, log);
  return Array.from(byId.values()).sort((a, b) => a.id - b.id);
}

export function statusFromExecutionLogs(
  logs: AgentExecutionLogRecord[],
  fallback: ActiveAgentExecution["status"],
): ActiveAgentExecution["status"] {
  const terminal = [...logs].reverse().find((log) =>
    log.phase === "completed" || log.phase === "failed" || log.phase === "halted"
      || log.phase === "restart_recovery_ready"
  );
  if (!terminal) return fallback;
  if (terminal.phase === "completed") return "completed";
  if (terminal.phase === "halted" || terminal.phase === "restart_recovery_ready") return "halted";
  return "failed";
}

export function shouldSynthesizeResumeActionKey(
  execution: ActiveAgentExecution | null,
  interruptedRecovery: { executionId: string; planId: string } | null,
) {
  if (!execution || execution.status !== "running") return false;
  return interruptedRecovery?.executionId !== execution.executionId
    || interruptedRecovery.planId !== execution.planId;
}


export function terminalExecutionStatusFromLogs(
  logs: AgentExecutionLogRecord[],
): ActiveAgentExecution["status"] {
  return statusFromExecutionLogs(logs, "failed");
}

export function executionStatusLabel(
  status: ActiveAgentExecution["status"],
  translate: (key: string) => string,
) {
  switch (status) {
    case "completed": return translate("chat.execution.status.complete");
    case "failed": return translate("chat.execution.status.failed");
    case "halted": return translate("chat.execution.status.halted");
    default: return translate("chat.execution.status.running");
  }
}

export function formatExecutionTime(value: number) {
  if (!value) return "";
  return new Intl.DateTimeFormat(undefined, {
    hour: "numeric",
    minute: "2-digit",
    second: "2-digit",
  }).format(new Date(value));
}
