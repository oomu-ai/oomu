import { useMemo } from "react";
import {
  clearStoredActiveExecution,
  persistActiveExecution,
  type ActiveAgentExecution,
} from "./agentExecutionState";
import {
  agentRecoveryActionKey,
  cancelRemainingAgentWorkForSession,
  checkCalendarFullAccessAndResumeForSession,
  checkMacPermissionAndResumeForExecution,
  checkMailAutomationAccessAndResumeForSession,
  openCalendarPrivacySettings,
  openMacPermissionSettings,
  openMailAutomationSettings,
  resolveAgentCalendarRecoveryForSession,
  resumeAgentExecutionForSession,
  type CalendarRecoveryResolution,
} from "./agentExecutionRecovery";
import { useStableEvent, type SessionScopedStateUpdate } from "./sessionScopedState";

type SetForSession<Value> = (
  sessionId: string,
  update: SessionScopedStateUpdate<Value>,
) => void;

type Translate = (key: string, variables?: Record<string, string | number>) => string;

type RecoveryHandlerOptions = {
  activeSessionId: string;
  setActiveExecution: SetForSession<ActiveAgentExecution | null>;
  setCompletedActions: SetForSession<ReadonlySet<string>>;
  setExecuting: SetForSession<boolean>;
  setProcessing: SetForSession<boolean>;
  setStatus: SetForSession<string>;
  translate: Translate;
};

function useMacPermissionRecoveryHandlers({
  activeSessionId,
  setActiveExecution,
  setCompletedActions,
  setExecuting,
  setProcessing,
  setStatus,
  translate,
}: RecoveryHandlerOptions) {
  const openPermissionSettings = useStableEvent(async (
    _executionId: string,
    capabilityId: string,
  ) => openMacPermissionSettings(capabilityId));
  const checkPermissionAccess = useStableEvent(async (
    executionId: string,
    capabilityId: string,
  ) => {
    const sessionId = activeSessionId.trim();
    const outcome = await checkMacPermissionAndResumeForExecution(executionId, capabilityId);
    if (outcome === "already_resumed") return;
    setActiveExecution(sessionId, (current) => current?.executionId === executionId
      ? { ...current, status: "running" }
      : current);
    setExecuting(sessionId, true);
    setProcessing(sessionId, true);
    setStatus(sessionId, translate("chat.status.executing_plan"));
    setCompletedActions(sessionId, (current) => new Set(current).add(
      agentRecoveryActionKey(executionId, "resume_same_execution"),
    ));
  });
  return { checkPermissionAccess, openPermissionSettings };
}

export function useAgentExecutionRecoveryHandlers({
  activeSessionId,
  setActiveExecution,
  setCompletedActions,
  setExecuting,
  setProcessing,
  setStatus,
  translate,
}: RecoveryHandlerOptions) {
  const { checkPermissionAccess, openPermissionSettings } =
    useMacPermissionRecoveryHandlers({
      activeSessionId, setActiveExecution, setCompletedActions, setExecuting,
      setProcessing, setStatus, translate,
    });
  const activateExecution = (
    sessionId: string,
    execution: ActiveAgentExecution,
    actionKey: string,
  ) => {
    persistActiveExecution(execution);
    setActiveExecution(sessionId, execution);
    setExecuting(sessionId, true);
    setProcessing(sessionId, true);
    setStatus(sessionId, translate("gateway.auto_turn.retrieving"));
    setCompletedActions(sessionId, (current) => new Set(current).add(actionKey));
  };

  const retry = useStableEvent(async (executionId: string) => {
    const sessionId = activeSessionId.trim();
    const execution = await resumeAgentExecutionForSession(executionId, sessionId);
    activateExecution(
      sessionId,
      execution,
      agentRecoveryActionKey(executionId, "resume_same_execution"),
    );
  });

  const openCalendarSettings = useStableEvent(async () => {
    await openCalendarPrivacySettings();
  });

  const checkCalendarAccess = useStableEvent(async (executionId: string) => {
    const sessionId = activeSessionId.trim();
    const execution = await checkCalendarFullAccessAndResumeForSession(executionId, sessionId);
    activateExecution(
      sessionId,
      execution,
      agentRecoveryActionKey(executionId, "resume_same_execution"),
    );
  });

  const openMailSettings = useStableEvent(async () => {
    await openMailAutomationSettings();
  });

  const checkMailAccess = useStableEvent(async (executionId: string) => {
    const sessionId = activeSessionId.trim();
    const execution = await checkMailAutomationAccessAndResumeForSession(executionId, sessionId);
    activateExecution(
      sessionId,
      execution,
      agentRecoveryActionKey(executionId, "resume_same_execution"),
    );
  });

  const cancelRemainingWork = useStableEvent(async (executionId: string) => {
    const sessionId = activeSessionId.trim();
    await cancelRemainingAgentWorkForSession(executionId, sessionId);
    setCompletedActions(sessionId, (current) => new Set(current)
      .add(agentRecoveryActionKey(executionId, "cancel_remaining_work")));
    setExecuting(sessionId, false);
    setProcessing(sessionId, false);
    clearStoredActiveExecution(sessionId);
    setActiveExecution(sessionId, null);
    setStatus(sessionId, translate("chat.recovery.calendar_permission_cancelled"));
  });

  const resolveCalendarRecovery = useStableEvent(async (
    executionId: string,
    choice: CalendarRecoveryResolution,
  ) => {
    const sessionId = activeSessionId.trim();
    const result = await resolveAgentCalendarRecoveryForSession(executionId, sessionId, choice);
    if (result.status === "cancelled") {
      setCompletedActions(sessionId, (current) => new Set(current)
        .add(agentRecoveryActionKey(executionId, "cancel_calendar_recovery")));
      setExecuting(sessionId, false);
      setProcessing(sessionId, false);
      setStatus(sessionId, translate("chat.recovery.calendar_cancelled"));
      return "cancelled" as const;
    }
    activateExecution(
      sessionId,
      result.execution,
      agentRecoveryActionKey(executionId, "resume_same_execution"),
    );
    setCompletedActions(sessionId, (current) => new Set(current)
      .add(agentRecoveryActionKey(executionId, "resolve_calendar_target")));
    return "resumed" as const;
  });

  return useMemo(() => ({
    onCancelRemainingWork: cancelRemainingWork,
    onCheckCalendarAccess: checkCalendarAccess,
    onCheckMacPermissionAccess: checkPermissionAccess,
    onCheckMailAutomationAccess: checkMailAccess,
    onOpenCalendarSettings: openCalendarSettings,
    onOpenMacPermissionSettings: openPermissionSettings,
    onOpenMailAutomationSettings: openMailSettings,
    onResolveCalendar: resolveCalendarRecovery,
    onRetry: retry,
  }), [
    cancelRemainingWork, checkCalendarAccess, checkMailAccess, checkPermissionAccess,
    openCalendarSettings, openMailSettings, openPermissionSettings, resolveCalendarRecovery, retry,
  ]);
}
