import { invoke, isTauriRuntime } from "@/lib/invoke";
import {
  executionIdFromStartResponse,
  planIdFromStartResponse,
  sessionIdFromStartResponse,
  streamStartAfterLogIdFromResponse,
  type ActiveAgentExecution,
  type AgentExecutionStartResponse,
} from "./agentExecutionState";

type AgentExecutionReplanResponse = {
  objective?: unknown;
};

export type CalendarRecoveryResolution =
  | { resolution: "select_existing"; calendarName: string }
  | { resolution: "create_requested" }
  | { resolution: "cancel" };

type CalendarRecoveryResponse = {
  status?: unknown;
  selectedCalendarName?: unknown;
};

type CalendarFullAccessResponse = {
  status?: unknown;
  fullAccess?: unknown;
  canRequestFullAccess?: unknown;
};

type MailAutomationAccessResponse = {
  status?: unknown;
  authorized?: unknown;
  retrySupported?: unknown;
};

type CancelRemainingAgentWorkResponse = {
  status?: unknown;
};

type MacPermissionStatus = {
  capabilityId?: unknown;
  state?: unknown;
};

type PermissionResumeResponse = {
  resumed?: unknown;
  executionId?: unknown;
  reason?: unknown;
};

export type AgentExecutionRecoveryState = {
  executionId: string;
  planId: string;
  sessionId?: string;
  rootTurnId?: string;
  failedTurnId?: string;
  generationToken?: string;
  status: "running" | "completed" | "failed" | "halted" | "cancelled";
  terminalPhase: string | null;
  terminalVerified: boolean;
  verifiedComplete: boolean;
};

const safeRecoveryIdentityPattern = /^[a-zA-Z0-9][a-zA-Z0-9_.:-]{0,127}$/;
const safeRecoveryPhasePattern = /^[a-z][a-z0-9_]{0,79}$/;
const recoveryStateBatchSize = 64;
const recoveryStatuses = new Set<AgentExecutionRecoveryState["status"]>([
  "running",
  "completed",
  "failed",
  "halted",
  "cancelled",
]);
const operationVerifiedPermissionCapabilities = new Set([
  "full_disk_access",
  "local_network",
  "mail",
  "notes",
  "messages",
  "finder",
  "system_events",
]);

export type AgentRecoveryPlanSubmissionOptions = {
  expectedSessionId: string;
  onAccepted: () => void;
  onPlanReady: () => void;
  recoveryPlan: true;
};

type AgentRecoveryPlanSubmit = (
  objective: string,
  options: AgentRecoveryPlanSubmissionOptions,
) => Promise<void>;

type AgentPlanSummary = {
  id: string;
  objective: string;
  steps: unknown[];
  trusted_automatic_execution?: boolean;
};

type Translate = (key: string, variables?: Record<string, string | number>) => string;

export function recoveryPlanRouteDecision(statusLabel: string) {
  return {
    route: "agentic_planner" as const,
    requires_local_access: true,
    decision_source: "recovery_replan",
    reason: "A stopped execution requires a fresh approval-gated plan.",
    matched_signals: ["recovery_replan"],
    status_label: statusLabel,
  };
}

function requiredRecoveryIdentity(value: string, errorCode: string) {
  const normalized = value.trim();
  if (!normalized || !isTauriRuntime) {
    throw new Error(errorCode);
  }
  return normalized;
}

function normalizeAgentExecutionRecoveryStates(
  value: unknown,
  requestedSessionId: string,
  requestedExecutionIds: ReadonlySet<string>,
): AgentExecutionRecoveryState[] {
  if (!Array.isArray(value)) throw new Error("agent_execution_recovery_states_invalid");
  const seen = new Set<string>();
  return value.map((entry) => {
    if (!entry || typeof entry !== "object" || Array.isArray(entry)) {
      throw new Error("agent_execution_recovery_states_invalid");
    }
    const record = entry as Record<string, unknown>;
    const executionId = typeof record.executionId === "string" ? record.executionId : "";
    const planId = typeof record.planId === "string" ? record.planId : "";
    const sessionId = typeof record.sessionId === "string" ? record.sessionId : "";
    const rootTurnId = typeof record.rootTurnId === "string" ? record.rootTurnId : "";
    const failedTurnId = typeof record.failedTurnId === "string" ? record.failedTurnId : "";
    const generationToken = typeof record.generationToken === "string"
      ? record.generationToken
      : "";
    const status = typeof record.status === "string" ? record.status : "";
    const terminalPhase = record.terminalPhase === null || record.terminalPhase === undefined
      ? null
      : typeof record.terminalPhase === "string" && safeRecoveryPhasePattern.test(record.terminalPhase)
        ? record.terminalPhase
        : undefined;
    if (
      !requestedExecutionIds.has(executionId)
      || seen.has(executionId)
      || !safeRecoveryIdentityPattern.test(executionId)
      || !safeRecoveryIdentityPattern.test(planId)
      || sessionId !== requestedSessionId
      || !safeRecoveryIdentityPattern.test(sessionId)
      || !safeRecoveryIdentityPattern.test(rootTurnId)
      || !safeRecoveryIdentityPattern.test(failedTurnId)
      || !safeRecoveryIdentityPattern.test(generationToken)
      || !recoveryStatuses.has(status as AgentExecutionRecoveryState["status"])
      || terminalPhase === undefined
      || typeof record.terminalVerified !== "boolean"
      || typeof record.verifiedComplete !== "boolean"
    ) {
      throw new Error("agent_execution_recovery_states_invalid");
    }
    seen.add(executionId);
    return {
      executionId,
      planId,
      sessionId,
      rootTurnId,
      failedTurnId,
      generationToken,
      status: status as AgentExecutionRecoveryState["status"],
      terminalPhase,
      terminalVerified: record.terminalVerified,
      verifiedComplete: record.verifiedComplete,
    };
  });
}

export async function getAgentExecutionRecoveryStates(
  sessionIdValue: string,
  executionIdValues: readonly string[],
) {
  const sessionId = requiredRecoveryIdentity(
    sessionIdValue,
    "agent_execution_recovery_states_unavailable",
  );
  const normalizedExecutionIds = executionIdValues.map((value) =>
    typeof value === "string" ? value.trim() : ""
  );
  const executionIds = Array.from(new Set(normalizedExecutionIds));
  if (
    !safeRecoveryIdentityPattern.test(sessionId)
    || executionIds.some((executionId) => !safeRecoveryIdentityPattern.test(executionId))
  ) {
    throw new Error("agent_execution_recovery_states_unavailable");
  }
  if (executionIds.length === 0) return [];

  const batches: string[][] = [];
  for (let index = 0; index < executionIds.length; index += recoveryStateBatchSize) {
    batches.push(executionIds.slice(index, index + recoveryStateBatchSize));
  }
  const responses = await Promise.all(batches.map(async (batch) => {
    const response = await invoke<unknown>("get_agent_execution_recovery_states", {
      sessionId,
      executionIds: batch,
    });
    return normalizeAgentExecutionRecoveryStates(response, sessionId, new Set(batch));
  }));
  return responses.flat();
}

export async function prepareAgentExecutionReplan(
  executionIdValue: string,
  sessionIdValue: string,
) {
  const executionId = requiredRecoveryIdentity(
    executionIdValue,
    "agent_execution_replan_unavailable",
  );
  const sessionId = requiredRecoveryIdentity(
    sessionIdValue,
    "agent_execution_replan_unavailable",
  );
  const response = await invoke<AgentExecutionReplanResponse>(
    "prepare_agent_execution_replan",
    { request: { executionId, sessionId } },
  );
  if (typeof response?.objective !== "string" || !response.objective.trim()) {
    throw new Error("agent_execution_replan_objective_missing");
  }
  return response.objective;
}

export async function startNewAgentRecoveryPlan(options: {
  executionId: string;
  sessionId: string;
  currentSessionId: () => string;
  submit: AgentRecoveryPlanSubmit;
}) {
  const objective = await prepareAgentExecutionReplan(
    options.executionId,
    options.sessionId,
  );
  if (options.currentSessionId().trim() !== options.sessionId) {
    throw new Error("agent_execution_replan_ownership_mismatch");
  }
  let accepted = false;
  let planReady = false;
  await options.submit(objective, {
    expectedSessionId: options.sessionId,
    onAccepted: () => { accepted = true; },
    onPlanReady: () => { planReady = true; },
    recoveryPlan: true,
  });
  if (!planReady) {
    throw new Error(accepted
      ? "agent_execution_replan_plan_missing"
      : "agent_execution_replan_submission_rejected");
  }
}

export function agentRecoveryActionKey(
  executionId: string,
  action: "resume_same_execution" | "resolve_calendar_target" | "cancel_calendar_recovery" | "cancel_remaining_work" | "start_new_plan",
) {
  return `${executionId}:${action}`;
}

export async function resolveAgentCalendarRecoveryForSession(
  executionIdValue: string,
  sessionIdValue: string,
  choice: CalendarRecoveryResolution,
): Promise<{ status: "cancelled"; execution: null } | { status: "resumed"; execution: ActiveAgentExecution }> {
  const executionId = requiredRecoveryIdentity(
    executionIdValue,
    "calendar_recovery_unavailable",
  );
  const sessionId = requiredRecoveryIdentity(
    sessionIdValue,
    "calendar_recovery_unavailable",
  );
  const response = await invoke<CalendarRecoveryResponse>(
    "resolve_agent_calendar_recovery",
    {
      request: {
        executionId,
        sessionId,
        resolution: choice.resolution,
        calendarName: "calendarName" in choice ? choice.calendarName : undefined,
      },
    },
  );
  if (response?.status === "cancelled") {
    return { status: "cancelled", execution: null };
  }
  if (
    response?.status !== "ready_to_resume" ||
    typeof response.selectedCalendarName !== "string" ||
    !response.selectedCalendarName.trim()
  ) {
    throw new Error("calendar_recovery_resolution_invalid");
  }
  return {
    status: "resumed",
    execution: await resumeAgentExecutionForSession(executionId, sessionId),
  };
}

export async function openCalendarPrivacySettings() {
  if (!isTauriRuntime) {
    throw new Error("calendar_settings_unavailable");
  }
  await invoke("open_calendar_privacy_settings");
}

export async function openMailAutomationSettings() {
  if (!isTauriRuntime) {
    throw new Error("mail_automation_settings_unavailable");
  }
  await invoke("open_mail_automation_settings");
}

export async function openMacPermissionSettings(capabilityIdValue: string) {
  const capabilityId = requiredRecoveryIdentity(
    capabilityIdValue,
    "mac_permission_settings_unavailable",
  );
  await invoke("open_macos_permission_settings", {
    request: { capabilityId },
  });
}

export async function checkMacPermissionAndResumeForExecution(
  executionIdValue: string,
  capabilityIdValue: string,
) {
  const executionId = requiredRecoveryIdentity(
    executionIdValue,
    "mac_permission_recovery_unavailable",
  );
  const capabilityId = requiredRecoveryIdentity(
    capabilityIdValue,
    "mac_permission_recovery_unavailable",
  );
  const statuses = await invoke<MacPermissionStatus[]>("list_macos_permission_states");
  const current = Array.isArray(statuses)
    ? statuses.find((entry) => entry.capabilityId === capabilityId)
    : null;
  const state = typeof current?.state === "string" ? current.state : "";
  if (
    !operationVerifiedPermissionCapabilities.has(capabilityId)
    && state !== "allowed"
    && state !== "limited"
    && state !== "when_used"
  ) {
    throw new Error("mac_permission_not_allowed");
  }
  const response = await invoke<PermissionResumeResponse>(
    "resume_agent_execution_after_permission",
    { request: { capabilityId, executionId } },
  );
  if (response?.resumed === true && response.executionId === executionId) {
    return "resumed" as const;
  }
  if (response?.reason === "already_resumed") {
    return "already_resumed" as const;
  }
  throw new Error("mac_permission_resume_unavailable");
}

export async function checkCalendarFullAccessAndResumeForSession(
  executionIdValue: string,
  sessionIdValue: string,
) {
  const executionId = requiredRecoveryIdentity(
    executionIdValue,
    "calendar_access_recovery_unavailable",
  );
  const sessionId = requiredRecoveryIdentity(
    sessionIdValue,
    "calendar_access_recovery_unavailable",
  );
  const access = await invoke<CalendarFullAccessResponse>("check_calendar_full_access");
  if (access?.status !== "full_access" || access.fullAccess !== true) {
    throw new Error("calendar_full_access_required");
  }
  return resumeAgentExecutionForSession(executionId, sessionId);
}

export async function checkMailAutomationAccessAndResumeForSession(
  executionIdValue: string,
  sessionIdValue: string,
) {
  const executionId = requiredRecoveryIdentity(
    executionIdValue,
    "mail_automation_recovery_unavailable",
  );
  const sessionId = requiredRecoveryIdentity(
    sessionIdValue,
    "mail_automation_recovery_unavailable",
  );
  const access = await invoke<MailAutomationAccessResponse>(
    "check_mail_automation_access",
  );
  const authorized = access?.status === "authorized" && access.authorized === true;
  const targetCanBeRetried = access?.status === "target_not_running"
    && access.retrySupported === true;
  if (!authorized && !targetCanBeRetried) {
    throw new Error("mail_automation_permission_required");
  }
  return resumeAgentExecutionForSession(executionId, sessionId);
}

export async function cancelRemainingAgentWorkForSession(
  executionIdValue: string,
  sessionIdValue: string,
) {
  const executionId = requiredRecoveryIdentity(
    executionIdValue,
    "agent_execution_cancel_unavailable",
  );
  const sessionId = requiredRecoveryIdentity(
    sessionIdValue,
    "agent_execution_cancel_unavailable",
  );
  const response = await invoke<CancelRemainingAgentWorkResponse>(
    "cancel_agent_execution_remaining_work",
    { request: { executionId, sessionId } },
  );
  if (response?.status !== "cancelled") {
    throw new Error("agent_execution_cancel_invalid");
  }
}

export function localizedAgentPlanSummary(
  translate: Translate,
  plan: AgentPlanSummary,
) {
  return [
    translate("chat.recovery.plan_compiled"),
    translate("chat.recovery.plan_id", { id: plan.id }),
    translate("chat.recovery.plan_objective", { objective: plan.objective }),
    plan.trusted_automatic_execution
      ? null
      : translate("chat.recovery.plan_steps", { count: plan.steps.length }),
  ].filter((line): line is string => Boolean(line)).join("\n");
}

export async function resumeAgentExecutionForSession(
  executionIdValue: string,
  sessionIdValue: string,
): Promise<ActiveAgentExecution> {
  const executionId = requiredRecoveryIdentity(
    executionIdValue,
    "agent_execution_resume_unavailable",
  );
  const sessionId = requiredRecoveryIdentity(
    sessionIdValue,
    "agent_execution_resume_unavailable",
  );
  const response = await invoke<AgentExecutionStartResponse>("resume_agent_execution", {
    request: { executionId },
  });
  const resumedExecutionId = executionIdFromStartResponse(response);
  const responseSessionId = sessionIdFromStartResponse(response, sessionId);
  const responsePlanId = planIdFromStartResponse(response, "");
  if (resumedExecutionId !== executionId || responseSessionId !== sessionId || !responsePlanId) {
    throw new Error("agent_execution_resume_ownership_mismatch");
  }
  const lastSeenId = streamStartAfterLogIdFromResponse(response);
  return {
    executionId: resumedExecutionId,
    planId: responsePlanId,
    sessionId: responseSessionId,
    status: "running",
    logs: [],
    lastSeenId,
    streamStartAfterLogId: lastSeenId,
    startedAtMs: Date.now(),
  };
}
