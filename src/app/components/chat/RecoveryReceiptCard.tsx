"use client";

import { useMemo, useState, type ReactElement } from "react";
import { useI18n } from "@/context/I18nContext";
import {
  agentRecoveryActionKey,
  type AgentExecutionRecoveryState,
  type CalendarRecoveryResolution,
} from "./agentExecutionRecovery";
import {
  macPermissionRecoveryDescriptor,
  MacPermissionRecoveryCard,
} from "./MacPermissionRecoveryCard";

const recoverySchema = "oomu.agent_execution_recovery.v1";
const safeTokenPattern = /^[a-z][a-z0-9_]{0,79}$/;
const safeExecutionIdPattern = /^[a-zA-Z0-9][a-zA-Z0-9_.:-]{0,127}$/;
const safeBoundaryPattern = /^[a-zA-Z][a-zA-Z0-9_]{0,79}$/;
const sha256Pattern = /^[a-f0-9]{64}$/;
const researchSubjects = new Set(["fuel", "freight"]);
const calendarPermissionRecoveryCodes = new Set([
  "calendar_authorization_timeout",
  "calendar_permission_denied",
  "calendar_permission_restricted",
  "calendar_permission_unavailable",
  "calendar_permission_write_only",
]);
const mailAutomationRecoveryCodes = new Set([
  "mail_automation_permission_required",
  "mail_automation_timeout",
  "mail_automation_unavailable",
]);

type RecoveryAction =
  | "resume_same_execution"
  | "resolve_calendar_target"
  | "calendar_recovery_cancelled"
  | "remaining_work_cancelled"
  | "start_new_plan"
  | "review_external_changes";

type AgentExecutionRecoveryReceipt = {
  schema: "oomu.agent_execution_recovery.v1";
  executionId: string;
  planId: string;
  code: string;
  boundary: string;
  recoverable: boolean;
  recoveryAction: RecoveryAction;
  message: string;
  changedState: "none" | "checkpoint_saved" | "external_changes";
  context: {
    subject: "fuel" | "freight" | null;
    verifiedInputs: number | null;
    attemptCount: number | null;
    pageCount: number | null;
    requestedCalendarName: string | null;
    availableCalendarNames: string[];
    nextOperation: string | null;
    frozenArgumentSha256: string | null;
    capabilityId: string | null;
  };
};

type RecoveryReceiptCardProps = {
  content: string;
  completedActionKeys?: ReadonlySet<string>;
  executionState?: AgentExecutionRecoveryState;
  executionStateStatus?: "idle" | "loading" | "ready" | "failed";
  recoveryReceiptAuthority?: "checking" | "current" | "inactive" | "unavailable";
  onRetry?: (executionId: string) => Promise<void>;
  onStartNewPlan?: (executionId: string) => Promise<void>;
  onResolveCalendar?: (
    executionId: string,
    choice: CalendarRecoveryResolution,
  ) => Promise<"resumed" | "cancelled">;
  onOpenCalendarSettings?: (executionId: string) => Promise<void>;
  onCheckCalendarAccess?: (executionId: string) => Promise<void>;
  onOpenMailAutomationSettings?: (executionId: string) => Promise<void>;
  onCheckMailAutomationAccess?: (executionId: string) => Promise<void>;
  onOpenMacPermissionSettings?: (executionId: string, capabilityId: string) => Promise<void>;
  onCheckMacPermissionAccess?: (executionId: string, capabilityId: string) => Promise<void>;
  onRefreshExecutionState?: () => void;
  onCancelRemainingWork?: (executionId: string) => Promise<void>;
};

export type RecoveryReceiptActions = Pick<
  RecoveryReceiptCardProps,
  | "onCancelRemainingWork"
  | "onCheckCalendarAccess"
  | "onCheckMacPermissionAccess"
  | "onCheckMailAutomationAccess"
  | "onOpenCalendarSettings"
  | "onOpenMacPermissionSettings"
  | "onOpenMailAutomationSettings"
  | "onResolveCalendar"
  | "onRetry"
>;

function record(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function token(value: unknown, fallback: string) {
  return typeof value === "string" && safeTokenPattern.test(value)
    ? value
    : fallback;
}

function boundedCount(value: unknown) {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0 && value <= 10_000
    ? value
    : null;
}

function boundedMessage(value: unknown) {
  if (typeof value !== "string" || !value.trim() || value.length > 1_000) return null;
  const normalized = value.trim().replace(/\s+/g, " ");
  return normalized.length > 360 ? `${normalized.slice(0, 357)}…` : normalized;
}

function boundedCalendarName(value: unknown) {
  if (typeof value !== "string") return null;
  const normalized = value.trim();
  return normalized && Array.from(normalized).length <= 80 ? normalized : null;
}

function boundedCalendarNames(value: unknown) {
  if (!Array.isArray(value)) return [];
  return Array.from(new Set(value.slice(0, 12).map(boundedCalendarName).filter(
    (name): name is string => Boolean(name),
  ))).sort((left, right) => left.localeCompare(right));
}

function boundedSha256(value: unknown) {
  return typeof value === "string" && sha256Pattern.test(value) ? value : null;
}

function recoveryErrorCode(error: unknown) {
  if (!error || typeof error !== "object") return "";
  const code = (error as { code?: unknown }).code;
  return typeof code === "string" && safeTokenPattern.test(code) ? code : "";
}

function parseEnvelope(content: string) {
  try {
    return record(JSON.parse(content.trim()));
  } catch {
    return null;
  }
}

function normalizedRecoveryAction(
  value: unknown,
  recoverable: boolean,
  changedState: AgentExecutionRecoveryReceipt["changedState"],
): RecoveryAction | null {
  const legacyAction: RecoveryAction = recoverable
    ? "resume_same_execution"
    : changedState === "none"
      ? "start_new_plan"
      : "review_external_changes";
  if (value === undefined) return legacyAction;
  if (
    value !== "resume_same_execution" &&
    value !== "resolve_calendar_target" &&
    value !== "calendar_recovery_cancelled" &&
    value !== "remaining_work_cancelled" &&
    value !== "start_new_plan" &&
    value !== "review_external_changes"
  ) {
    return null;
  }
  const compatible = value === "resume_same_execution"
    ? recoverable
    : value === "resolve_calendar_target"
      ? recoverable && changedState !== "external_changes"
    : value === "calendar_recovery_cancelled"
      ? !recoverable && changedState !== "external_changes"
    : value === "remaining_work_cancelled"
      ? !recoverable && changedState !== "external_changes"
    : value === "start_new_plan"
      ? !recoverable && changedState === "none"
      : !recoverable && changedState !== "none";
  return compatible ? value : null;
}

export function parseAgentExecutionRecoveryReceipt(
  content: string,
): AgentExecutionRecoveryReceipt | null {
  const envelope = parseEnvelope(content);
  if (!envelope) return null;
  if (envelope.schema !== recoverySchema) return null;
  const executionId = envelope.executionId;
  if (typeof executionId !== "string" || !safeExecutionIdPattern.test(executionId)) return null;
  const planId = envelope.planId;
  if (typeof planId !== "string" || !safeExecutionIdPattern.test(planId)) return null;
  if (typeof envelope.boundary !== "string" || !safeBoundaryPattern.test(envelope.boundary)) return null;
  const message = boundedMessage(envelope.message);
  if (!message) return null;
  if (typeof envelope.recoverable !== "boolean") return null;
  const code = token(envelope.code, "");
  if (!code) return null;
  const changedState = envelope.changedState;
  if (changedState !== "none" && changedState !== "checkpoint_saved" && changedState !== "external_changes") {
    return null;
  }
  const recoveryAction = normalizedRecoveryAction(
    envelope.recoveryAction,
    envelope.recoverable,
    changedState,
  );
  if (!recoveryAction) return null;
  const context = record(envelope.context) ?? {};
  const rawSubject = token(context.subject, "");

  return {
    schema: "oomu.agent_execution_recovery.v1",
    executionId,
    planId,
    code,
    boundary: envelope.boundary,
    recoverable: envelope.recoverable,
    recoveryAction,
    message,
    changedState,
    context: {
      subject: researchSubjects.has(rawSubject)
        ? rawSubject as "fuel" | "freight"
        : null,
      verifiedInputs: boundedCount(context.verifiedInputCount),
      attemptCount: boundedCount(context.attemptCount),
      pageCount: boundedCount(context.pageCount),
      requestedCalendarName: boundedCalendarName(context.requestedCalendarName),
      availableCalendarNames: boundedCalendarNames(context.availableCalendarNames),
      nextOperation: token(context.nextOperation, "") || null,
      frozenArgumentSha256: boundedSha256(context.frozenArgumentSha256),
      capabilityId: token(context.capabilityId, "") || null,
    },
  };
}

export function resumablePermissionCapability(
  content: string,
  executionId: string,
) {
  const receipt = parseAgentExecutionRecoveryReceipt(content);
  if (
    !receipt ||
    receipt.executionId !== executionId ||
    !receipt.recoverable ||
    receipt.recoveryAction !== "resume_same_execution"
  ) {
    return null;
  }
  if (receipt.context.capabilityId) return receipt.context.capabilityId;
  if (receipt.code.startsWith("calendar_permission_") || receipt.code === "calendar_authorization_timeout") {
    return "calendar";
  }
  if (receipt.code.startsWith("contacts_permission_") || receipt.code.startsWith("contacts_authorization_")) {
    return "contacts";
  }
  return receipt.code === "mail_automation_permission_required" ? "mail" : null;
}

function isResearchRecovery(receipt: AgentExecutionRecoveryReceipt) {
  return receipt.code.startsWith("decision_pack_research_")
    || receipt.context.subject !== null;
}

export function isCalendarPermissionRecoveryCode(code: string) {
  return calendarPermissionRecoveryCodes.has(code);
}

export function isMailAutomationRecoveryCode(code: string) {
  return mailAutomationRecoveryCodes.has(code);
}

function calendarPermissionBodyKey(code: string) {
  if (code === "calendar_permission_write_only") {
    return "chat.recovery.calendar_permission_write_only_body";
  }
  if (code === "calendar_permission_unavailable") {
    return "chat.recovery.calendar_permission_unavailable_body";
  }
  if (code === "calendar_authorization_timeout") {
    return "chat.recovery.calendar_permission_timeout_body";
  }
  return "chat.recovery.calendar_permission_denied_body";
}

function calendarActionFailureBodyKey(code: string) {
  if (code === "calendar_not_found") return "chat.recovery.calendar_missing_body";
  if (code === "calendar_name_ambiguous") return "chat.recovery.calendar_ambiguous_body";
  if (code === "calendar_read_only") return "chat.recovery.calendar_read_only_body";
  if (code === "calendar_availability_unsupported") {
    return "chat.recovery.calendar_incompatible_body";
  }
  if (code === "calendar_permission_write_only") {
    return "chat.recovery.calendar_permission_write_only_body";
  }
  if (calendarPermissionRecoveryCodes.has(code)) {
    return "chat.recovery.calendar_permission_denied_body";
  }
  if (code === "calendar_cleanup_failed") return "chat.recovery.review_body";
  return "chat.recovery.calendar_failed";
}

function mailAutomationTitleKey(code: string) {
  if (code === "mail_automation_timeout") {
    return "chat.recovery.mail_automation_timeout_title";
  }
  if (code === "mail_automation_unavailable") {
    return "chat.recovery.mail_automation_unavailable_title";
  }
  return "chat.recovery.mail_automation_permission_title";
}

function mailAutomationBodyKey(code: string, sendsEmail: boolean) {
  if (sendsEmail) {
    if (code === "mail_automation_timeout") {
      return "chat.recovery.mail_send_timeout_body";
    }
    if (code === "mail_automation_unavailable") {
      return "chat.recovery.mail_send_unavailable_body";
    }
    return "chat.recovery.mail_send_permission_body";
  }
  if (code === "mail_automation_timeout") {
    return "chat.recovery.mail_automation_timeout_body";
  }
  if (code === "mail_automation_unavailable") {
    return "chat.recovery.mail_automation_unavailable_body";
  }
  return "chat.recovery.mail_automation_permission_body";
}

function boundaryTranslationKey(boundary: string) {
  const normalized = boundary.replace(/([a-z0-9])([A-Z])/g, "$1_$2").toLowerCase();
  const known = new Set(["decision_pack", "research", "calendar", "mail", "verification"]);
  return known.has(normalized)
    ? `chat.recovery.boundaries.${normalized}`
    : "chat.recovery.boundaries.execution";
}

function genericMacPermissionRecoveryCard(
  receipt: AgentExecutionRecoveryReceipt,
  t: (key: string, variables?: Record<string, string | number>) => string,
  onCancel: RecoveryReceiptCardProps["onCancelRemainingWork"],
  onCheck: RecoveryReceiptCardProps["onCheckMacPermissionAccess"],
  onOpenSettings: RecoveryReceiptCardProps["onOpenMacPermissionSettings"],
) {
  const descriptor = macPermissionRecoveryDescriptor(
    receipt.code,
    receipt.context.capabilityId,
  );
  if (!descriptor || descriptor.capabilityId === "calendar" || descriptor.capabilityId === "mail") {
    return null;
  }
  return (
    <MacPermissionRecoveryCard
      boundary={receipt.boundary}
      code={receipt.code}
      descriptor={descriptor}
      recoveryId={receipt.executionId}
      onCancel={onCancel}
      onCheck={onCheck}
      onOpenSettings={onOpenSettings}
      t={t}
    />
  );
}

function calendarTargetIsResolved(receipt: AgentExecutionRecoveryReceipt) {
  return receipt.code === "calendar_target_resolved"
    && receipt.recoveryAction === "resume_same_execution";
}

function preferPermissionRecovery(
  permissionCard: ReactElement | null,
  defaultCard: ReactElement,
) {
  return permissionCard ?? defaultCard;
}

export function RecoveryReceiptCard({
  content,
  completedActionKeys,
  executionState,
  executionStateStatus = "idle",
  recoveryReceiptAuthority,
  onRetry, onStartNewPlan,
  onResolveCalendar, onOpenCalendarSettings,
  onCheckCalendarAccess, onOpenMailAutomationSettings,
  onCheckMailAutomationAccess, onOpenMacPermissionSettings,
  onCheckMacPermissionAccess,
  onRefreshExecutionState,
  onCancelRemainingWork,
}: RecoveryReceiptCardProps) {
  const { t } = useI18n();
  const receipt = useMemo(() => parseAgentExecutionRecoveryReceipt(content), [content]);
  const [actionState, setActionState] = useState<"idle" | "running" | "failed" | "completed">("idle");
  const [calendarOutcome, setCalendarOutcome] = useState<"resumed" | "cancelled" | null>(null);
  const [calendarPermissionState, setCalendarPermissionState] = useState<
    "idle" | "opening" | "checking" | "cancelling" | "continued" | "cancelled" | "failed"
  >("idle");
  const [mailAutomationState, setMailAutomationState] = useState<
    "idle" | "opening" | "checking" | "retrying" | "cancelling" | "continued" | "cancelled" | "failed"
  >("idle");
  const [selectedCalendar, setSelectedCalendar] = useState("");
  const [calendarFailureCode, setCalendarFailureCode] = useState("");

  if (!receipt) return null;
  const permissionCard = genericMacPermissionRecoveryCard(receipt, t, onCancelRemainingWork, onCheckMacPermissionAccess, onOpenMacPermissionSettings);
  const calendarTargetResolved = calendarTargetIsResolved(receipt);
  const calendarActionDenied = receipt.code === "calendar_action_denied";
  const calendarTargetRecovery = receipt.recoveryAction === "resolve_calendar_target"
    || calendarTargetResolved
    || receipt.recoveryAction === "calendar_recovery_cancelled";
  const remainingWorkCancellationReceipt = receipt.recoveryAction === "remaining_work_cancelled";
  const calendarPermissionRecovery = isCalendarPermissionRecoveryCode(receipt.code);
  const calendarActionNeedsFullAccess = calendarTargetRecovery
    && isCalendarPermissionRecoveryCode(calendarFailureCode);
  const mailAutomationRecovery = isMailAutomationRecoveryCode(receipt.code);
  const mailAutomationSendsEmail =
    receipt.context.nextOperation === "send_system_email";
  const finalVerificationRecovery = receipt.code === "mlc_verification_failed"
    && receipt.recoveryAction === "resume_same_execution";
  const interruptedMailApproval = receipt.code === "agent_execution_interrupted"
    && receipt.recoveryAction === "resume_same_execution"
    && receipt.context.nextOperation === "draft_release_recovery_email"
    && receipt.context.frozenArgumentSha256 !== null;
  const executionStateMatches = executionState?.executionId === receipt.executionId
    && executionState.planId === receipt.planId;
  const recoveryAuthorityCurrent = recoveryReceiptAuthority === undefined
    || recoveryReceiptAuthority === "current";
  const interruptedMailCanRestore = interruptedMailApproval
    && recoveryAuthorityCurrent
    && executionStateStatus === "ready"
    && executionStateMatches
    && executionState.status === "halted"
    && executionState.terminalPhase === "restart_recovery_ready";
  const interruptedMailCompleted = interruptedMailApproval
    && executionStateStatus === "ready"
    && executionStateMatches
    && executionState.status === "completed"
    && executionState.verifiedComplete;
  const interruptedMailRunning = interruptedMailApproval
    && executionStateStatus === "ready"
    && executionStateMatches
    && executionState.status === "running";
  const interruptedMailInactive = interruptedMailApproval
    && (recoveryReceiptAuthority === "inactive"
      || (recoveryAuthorityCurrent
        && executionStateStatus === "ready"
        && !interruptedMailCanRestore
        && !interruptedMailCompleted
        && !interruptedMailRunning));
  const interruptedMailStateFailed = interruptedMailApproval
    && executionStateStatus === "failed";
  const interruptedMailStateChecking = interruptedMailApproval
    && (executionStateStatus === "idle" || executionStateStatus === "loading");
  const recoveryAuthorityChecking = recoveryReceiptAuthority === "checking"
    && !interruptedMailCompleted;
  const recoveryAuthorityInactive = recoveryReceiptAuthority === "inactive"
    && !interruptedMailCompleted;
  const recoveryAuthorityUnavailable = recoveryReceiptAuthority === "unavailable"
    && !interruptedMailCompleted;
  const calendarRecovery = calendarTargetRecovery || calendarPermissionRecovery;
  const cancelledActionCompleted = calendarTargetRecovery && Boolean(completedActionKeys?.has(
    agentRecoveryActionKey(receipt.executionId, "cancel_calendar_recovery"),
  ));
  const remainingWorkCancelled = (calendarPermissionRecovery || mailAutomationRecovery) && Boolean(completedActionKeys?.has(
    agentRecoveryActionKey(receipt.executionId, "cancel_remaining_work"),
  ));
  const actionKey = receipt.recoveryAction === "review_external_changes"
    || receipt.recoveryAction === "calendar_recovery_cancelled"
    || receipt.recoveryAction === "remaining_work_cancelled"
    ? null
    : agentRecoveryActionKey(receipt.executionId, receipt.recoveryAction);
  const actionCompleted = receipt.recoveryAction === "calendar_recovery_cancelled"
    || remainingWorkCancellationReceipt
    || actionState === "completed"
    || calendarPermissionState === "continued"
    || calendarPermissionState === "cancelled"
    || mailAutomationState === "continued"
    || mailAutomationState === "cancelled"
    || interruptedMailCompleted
    || interruptedMailRunning
    || interruptedMailInactive
    || cancelledActionCompleted
    || remainingWorkCancelled || Boolean(
    actionKey && completedActionKeys?.has(actionKey),
  );
  const calendarResolutionSucceeded = calendarTargetResolved
    || (calendarTargetRecovery && actionCompleted
      && receipt.recoveryAction !== "calendar_recovery_cancelled"
      && calendarOutcome !== "cancelled");
  const recoverySucceeded = (calendarResolutionSucceeded && recoveryAuthorityCurrent)
    || interruptedMailCompleted;
  const researchRecovery = isResearchRecovery(receipt);
  const subject = receipt.context.subject
    ? t(`chat.recovery.subjects.${receipt.context.subject}`)
    : t("chat.recovery.subjects.research");
  const requestedCalendar = receipt.context.requestedCalendarName ?? t("chat.recovery.calendar_unknown");
  const title = recoveryAuthorityChecking
    ? t("chat.recovery.recovery_authority_checking_title")
    : recoveryAuthorityUnavailable
    ? t("chat.recovery.recovery_authority_unavailable_title")
    : recoveryAuthorityInactive
    ? interruptedMailApproval
      ? t("chat.recovery.interrupted_mail_inactive_title")
      : t("chat.recovery.recovery_authority_inactive_title")
    : remainingWorkCancellationReceipt
    ? t("chat.recovery.calendar_permission_cancelled_title")
    : calendarPermissionRecovery
    ? t("chat.recovery.calendar_permission_title")
    : mailAutomationRecovery
    ? t(mailAutomationTitleKey(receipt.code))
    : interruptedMailCompleted
    ? t("chat.recovery.interrupted_mail_completed_title")
    : interruptedMailInactive
    ? t("chat.recovery.interrupted_mail_inactive_title")
    : interruptedMailStateFailed
    ? t("chat.recovery.generic_title")
    : interruptedMailApproval
    ? t("chat.recovery.interrupted_mail_title")
    : calendarActionDenied
    ? t("chat.recovery.calendar_action_denied_title")
    : calendarTargetRecovery
    ? t(calendarResolutionSucceeded
      ? "chat.recovery.calendar_ready_title"
      : "chat.recovery.calendar_title")
    : finalVerificationRecovery
    ? t("chat.recovery.verification_title")
    : researchRecovery
    ? t("chat.recovery.research_title", { subject })
    : t("chat.recovery.generic_title");
  const body = recoveryAuthorityChecking
    ? t("chat.recovery.recovery_authority_checking_body")
    : recoveryAuthorityUnavailable
    ? t("chat.recovery.recovery_authority_unavailable_body")
    : recoveryAuthorityInactive
    ? interruptedMailApproval
      ? t("chat.recovery.interrupted_mail_inactive_body")
      : t("chat.recovery.recovery_authority_inactive_body")
    : remainingWorkCancellationReceipt
    ? t("chat.recovery.calendar_permission_cancelled")
    : calendarPermissionRecovery
    ? t(calendarPermissionBodyKey(receipt.code))
    : mailAutomationRecovery
    ? t(mailAutomationBodyKey(receipt.code, mailAutomationSendsEmail))
    : interruptedMailCompleted
    ? t("chat.recovery.interrupted_mail_completed_body")
    : interruptedMailInactive
    ? t("chat.recovery.interrupted_mail_inactive_body")
    : interruptedMailStateFailed
    ? t("chat.recovery.generic_body")
    : interruptedMailApproval
    ? t("chat.recovery.interrupted_mail_body")
    : calendarTargetRecovery
    ? calendarResolutionSucceeded
      ? t("chat.recovery.calendar_resolved")
      : receipt.recoveryAction === "calendar_recovery_cancelled"
      ? t("chat.recovery.calendar_cancelled")
      : calendarActionDenied
      ? t("chat.recovery.calendar_action_denied_body", { calendar: requestedCalendar })
      : receipt.code === "calendar_name_ambiguous"
      ? t("chat.recovery.calendar_ambiguous_body", { calendar: requestedCalendar })
      : receipt.code === "calendar_read_only"
      ? t("chat.recovery.calendar_read_only_body", { calendar: requestedCalendar })
      : receipt.code === "calendar_availability_unsupported"
      ? t("chat.recovery.calendar_incompatible_body", { calendar: requestedCalendar })
      : t("chat.recovery.calendar_missing_body", { calendar: requestedCalendar })
    : finalVerificationRecovery
    ? t("chat.recovery.verification_body")
    : receipt.recoveryAction === "start_new_plan"
    ? t("chat.recovery.start_new_plan_body")
    : receipt.recoveryAction === "review_external_changes"
      ? t("chat.recovery.review_body")
      : researchRecovery
        ? receipt.context.verifiedInputs !== null && receipt.context.verifiedInputs > 0
          ? t("chat.recovery.research_body_verified_inputs", {
              count: receipt.context.verifiedInputs,
              subject,
            })
          : t("chat.recovery.research_body", { subject })
        : t("chat.recovery.generic_body");
  const stateKey = recoveryAuthorityChecking
    ? "chat.recovery.recovery_authority_checking_state"
    : recoveryAuthorityUnavailable
    ? "chat.recovery.checkpoint_saved"
    : recoveryAuthorityInactive
    ? "chat.recovery.recovery_authority_inactive_state"
    : actionCompleted
    ? remainingWorkCancellationReceipt
      ? "chat.recovery.calendar_permission_cancelled"
      : calendarPermissionRecovery
      ? remainingWorkCancellationReceipt || calendarPermissionState === "cancelled" || remainingWorkCancelled
        ? "chat.recovery.calendar_permission_cancelled"
        : "chat.recovery.calendar_permission_ready"
      : mailAutomationRecovery
      ? mailAutomationState === "cancelled" || remainingWorkCancelled
        ? "chat.recovery.mail_automation_cancelled"
        : "chat.recovery.mail_automation_ready"
      : interruptedMailCompleted
      ? "chat.recovery.verification_ready"
      : interruptedMailRunning
      ? "chat.recovery.interrupted_mail_restoring"
      : interruptedMailInactive
      ? "chat.recovery.recovery_authority_inactive_state"
      : calendarTargetRecovery
      ? receipt.recoveryAction === "calendar_recovery_cancelled" || calendarOutcome === "cancelled" || cancelledActionCompleted
        ? "chat.recovery.calendar_cancelled"
        : "chat.recovery.resumed"
      : receipt.recoveryAction === "start_new_plan"
      ? "chat.recovery.plan_ready"
      : "chat.recovery.resumed"
    : receipt.recoveryAction === "review_external_changes"
    ? "chat.recovery.review_required"
    : finalVerificationRecovery
    ? "chat.recovery.verification_ready"
    : interruptedMailStateChecking
    ? "chat.recovery.interrupted_mail_checking_state"
    : interruptedMailStateFailed
    ? "chat.recovery.checkpoint_saved"
    : interruptedMailApproval
    ? "chat.recovery.interrupted_mail_state"
    : calendarActionDenied
    ? "chat.recovery.calendar_action_denied_state"
    : receipt.changedState === "checkpoint_saved"
      ? "chat.recovery.checkpoint_saved"
      : receipt.changedState === "external_changes"
        ? "chat.recovery.external_changes"
        : "chat.recovery.nothing_changed";
  const actionHandler = !recoveryAuthorityCurrent
    || actionCompleted
    || (calendarRecovery && !calendarTargetResolved)
    || mailAutomationRecovery
    || (interruptedMailApproval && !interruptedMailCanRestore)
    ? undefined
    : receipt.recoveryAction === "resume_same_execution"
    ? onRetry
    : receipt.recoveryAction === "start_new_plan"
      ? onStartNewPlan
      : undefined;
  const actionLabel = interruptedMailApproval
    ? t("chat.recovery.interrupted_mail_restore")
    : receipt.recoveryAction === "start_new_plan"
    ? t("chat.recovery.start_new_plan")
    : calendarTargetResolved
    ? t("chat.recovery.calendar_continue")
    : finalVerificationRecovery
    ? t("chat.recovery.verify_existing")
    : researchRecovery
    ? t("chat.recovery.retry_research")
    : t("chat.recovery.retry");
  const runningLabel = interruptedMailApproval
    ? t("chat.recovery.interrupted_mail_restoring")
    : receipt.recoveryAction === "start_new_plan"
    ? t("chat.recovery.starting_new_plan")
    : calendarTargetResolved
    ? t("chat.recovery.calendar_continuing")
    : finalVerificationRecovery
    ? t("chat.recovery.verifying_existing")
    : t("chat.recovery.retrying");

  async function performAction() {
    if (!recoveryAuthorityCurrent || !actionHandler || actionState === "running") return;
    setActionState("running");
    try {
      await actionHandler(receipt!.executionId);
      setActionState("completed");
    } catch {
      setActionState("failed");
    }
  }

  async function resolveCalendar(choice: CalendarRecoveryResolution) {
    if (!recoveryAuthorityCurrent || !onResolveCalendar || actionState === "running") return;
    setCalendarFailureCode("");
    setActionState("running");
    try {
      const outcome = await onResolveCalendar(receipt!.executionId, choice);
      setCalendarOutcome(outcome);
      setActionState("completed");
    } catch (error) {
      setCalendarFailureCode(recoveryErrorCode(error));
      setActionState("failed");
    }
  }

  async function performCalendarPermissionAction(
    action: "open" | "check" | "cancel",
  ) {
    const handler = action === "open"
      ? onOpenCalendarSettings
      : action === "check"
        ? onCheckCalendarAccess ?? onRetry
        : onCancelRemainingWork;
    if (
      !recoveryAuthorityCurrent
      || !handler
      || !receipt
      || !["idle", "failed"].includes(calendarPermissionState)
    ) return;
    setCalendarPermissionState(action === "open"
      ? "opening"
      : action === "check"
        ? "checking"
        : "cancelling");
    try {
      await handler(receipt.executionId);
      setCalendarPermissionState(action === "open"
        ? "idle"
        : action === "check"
          ? "continued"
          : "cancelled");
    } catch {
      setCalendarPermissionState("failed");
    }
  }

  async function performMailAutomationAction(
    action: "open" | "check" | "retry" | "cancel",
  ) {
    const handler = action === "open"
      ? onOpenMailAutomationSettings
      : action === "check"
        ? onCheckMailAutomationAccess
        : action === "retry"
          ? onRetry
          : onCancelRemainingWork;
    if (
      !recoveryAuthorityCurrent
      || !handler
      || !receipt
      || !["idle", "failed"].includes(mailAutomationState)
    ) return;
    setMailAutomationState(action === "open"
      ? "opening"
      : action === "check"
        ? "checking"
        : action === "retry"
          ? "retrying"
          : "cancelling");
    try {
      await handler(receipt.executionId);
      setMailAutomationState(action === "open"
        ? "idle"
        : action === "cancel"
          ? "cancelled"
          : "continued");
    } catch {
      setMailAutomationState("failed");
    }
  }

  return preferPermissionRecovery(permissionCard, (
    <section
      aria-label={title}
      className={`max-w-3xl self-start rounded-[var(--radius-lg)] border px-5 py-4 text-[var(--foreground)] ${recoverySucceeded
        ? "border-[var(--success)]/30 bg-[var(--success-background)]"
        : "border-[var(--warning)] bg-[var(--warning-background)]"}`}
      data-calendar-recovery-requested={calendarTargetRecovery ? receipt.context.requestedCalendarName ?? undefined : undefined}
      data-oomu-calendar-recovery-action={!recoveryAuthorityCurrent
        ? undefined
        : calendarPermissionRecovery
        ? "restore_calendar_full_access"
        : calendarTargetRecovery
          ? calendarResolutionSucceeded ? "calendar_target_resolved" : "resolve_calendar_target"
          : undefined}
      data-oomu-calendar-recovery-code={calendarRecovery ? receipt.code : undefined}
      data-oomu-mail-recovery-action={!recoveryAuthorityCurrent
        ? undefined
        : mailAutomationRecovery
        ? receipt.code === "mail_automation_permission_required"
          ? "restore_mail_automation_access"
          : "retry_mail_automation"
        : undefined}
      data-oomu-mail-recovery-code={mailAutomationRecovery ? receipt.code : undefined}
      data-oomu-interrupted-approval={interruptedMailCanRestore ? "mail_draft" : undefined}
      data-oomu-recovery-execution-id={receipt.executionId}
      data-oomu-verification-recovery={finalVerificationRecovery ? "verify_existing" : undefined}
    >
      <div className="flex items-start gap-3">
        <span aria-hidden="true" className={`mt-0.5 inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-[var(--background)] ${recoverySucceeded ? "text-[var(--success)]" : "text-[var(--warning)]"}`}>
          <svg className="h-4 w-4" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.8" viewBox="0 0 24 24">
            {recoverySucceeded ? (
              <path d="m5 12 4 4L19 6" />
            ) : (
              <><path d="M12 3a9 9 0 1 0 9 9" /><path d="M12 7v5l3 2M17 3h4v4" /></>
            )}
          </svg>
        </span>
        <div className="min-w-0 flex-1">
          <h3 className="text-sm font-semibold">{title}</h3>
          <p className="mt-1 text-sm leading-6 text-[var(--foreground-muted)]">{body}</p>
          <p className="mt-2 text-xs font-medium text-[var(--foreground)]">{t(stateKey)}</p>

          <div className="mt-4 flex flex-wrap items-center gap-3">
            {recoveryAuthorityUnavailable ? (
              <button
                className="rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-3.5 py-2 text-xs font-semibold text-[var(--inverse-foreground)] disabled:cursor-not-allowed disabled:opacity-50"
                disabled={!onRefreshExecutionState}
                onClick={onRefreshExecutionState}
                type="button"
              >
                {t("chat.recovery.recovery_authority_refresh")}
              </button>
            ) : recoveryAuthorityChecking || recoveryAuthorityInactive ? null
            : interruptedMailStateFailed && !actionCompleted ? (
              <button
                className="rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-3.5 py-2 text-xs font-semibold text-[var(--inverse-foreground)] disabled:cursor-not-allowed disabled:opacity-50"
                disabled={!onRefreshExecutionState}
                onClick={onRefreshExecutionState}
                type="button"
              >
                {t("chat.recovery.retry")}
              </button>
            ) : calendarPermissionRecovery && !actionCompleted ? (
              <div className="flex w-full flex-wrap gap-2">
                <button
                  className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3.5 py-2 text-xs font-semibold disabled:cursor-not-allowed disabled:opacity-50"
                  data-calendar-permission-action="open_settings"
                  disabled={!onOpenCalendarSettings || !["idle", "failed"].includes(calendarPermissionState)}
                  onClick={() => void performCalendarPermissionAction("open")}
                  type="button"
                >
                  {t("chat.recovery.calendar_permission_open_settings")}
                </button>
                <button
                  className="rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-3.5 py-2 text-xs font-semibold text-[var(--inverse-foreground)] disabled:cursor-not-allowed disabled:opacity-50"
                  data-calendar-permission-action="check_and_continue"
                  disabled={!(onCheckCalendarAccess ?? onRetry) || !["idle", "failed"].includes(calendarPermissionState)}
                  onClick={() => void performCalendarPermissionAction("check")}
                  type="button"
                >
                  {calendarPermissionState === "checking"
                    ? t("chat.recovery.calendar_continuing")
                    : t("chat.recovery.calendar_permission_check")}
                </button>
                <button
                  className="rounded-[var(--radius-sm)] px-3.5 py-2 text-xs font-semibold text-[var(--foreground-muted)] disabled:cursor-not-allowed disabled:opacity-50"
                  data-calendar-permission-action="cancel_remaining"
                  disabled={!onCancelRemainingWork || !["idle", "failed"].includes(calendarPermissionState)}
                  onClick={() => void performCalendarPermissionAction("cancel")}
                  type="button"
                >
                  {t("chat.recovery.calendar_permission_cancel_remaining")}
                </button>
              </div>
            ) : mailAutomationRecovery && !actionCompleted ? (
              <div className="flex w-full flex-wrap gap-2">
                {receipt.code === "mail_automation_permission_required" ? (
                  <button
                    className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3.5 py-2 text-xs font-semibold disabled:cursor-not-allowed disabled:opacity-50"
                    data-mail-automation-action="open_settings"
                    disabled={!onOpenMailAutomationSettings || !["idle", "failed"].includes(mailAutomationState)}
                    onClick={() => void performMailAutomationAction("open")}
                    type="button"
                  >
                    {t("chat.recovery.mail_automation_open_settings")}
                  </button>
                ) : null}
                <button
                  className="rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-3.5 py-2 text-xs font-semibold text-[var(--inverse-foreground)] disabled:cursor-not-allowed disabled:opacity-50"
                  data-mail-automation-action={receipt.code === "mail_automation_permission_required"
                    ? "check_and_continue"
                    : "retry_and_continue"}
                  disabled={receipt.code === "mail_automation_permission_required"
                    ? !onCheckMailAutomationAccess || !["idle", "failed"].includes(mailAutomationState)
                    : !onRetry || !["idle", "failed"].includes(mailAutomationState)}
                  onClick={() => void performMailAutomationAction(
                    receipt.code === "mail_automation_permission_required" ? "check" : "retry",
                  )}
                  type="button"
                >
                  {mailAutomationState === "checking" || mailAutomationState === "retrying"
                    ? t("chat.recovery.mail_automation_continuing")
                    : receipt.code === "mail_automation_permission_required"
                      ? t("chat.recovery.mail_automation_check")
                      : t("chat.recovery.mail_automation_retry")}
                </button>
                <button
                  className="rounded-[var(--radius-sm)] px-3.5 py-2 text-xs font-semibold text-[var(--foreground-muted)] disabled:cursor-not-allowed disabled:opacity-50"
                  data-mail-automation-action="cancel_remaining"
                  disabled={!onCancelRemainingWork || !["idle", "failed"].includes(mailAutomationState)}
                  onClick={() => void performMailAutomationAction("cancel")}
                  type="button"
                >
                  {t("chat.recovery.mail_automation_cancel_remaining")}
                </button>
              </div>
            ) : calendarTargetRecovery && !calendarTargetResolved && !actionCompleted ? (
              <div className="flex w-full flex-col gap-3">
                {receipt.context.availableCalendarNames.length > 0 ? (
                  <label className="flex max-w-md flex-col gap-1.5 text-xs font-semibold">
                    {t("chat.recovery.calendar_choose")}
                    <select
                      className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-sm font-medium"
                      disabled={actionState === "running"}
                      data-calendar-recovery-action="select_existing"
                      onChange={(event) => {
                        setSelectedCalendar(event.target.value);
                        setCalendarFailureCode("");
                        if (actionState === "failed") setActionState("idle");
                      }}
                      value={selectedCalendar}
                    >
                      <option value="">{t("chat.recovery.calendar_choose_placeholder")}</option>
                      {receipt.context.availableCalendarNames.map((name) => (
                        <option key={name} value={name}>{name}</option>
                      ))}
                    </select>
                  </label>
                ) : null}
                <div className="flex flex-wrap gap-2">
                  {calendarActionNeedsFullAccess ? (
                    <button
                      className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3.5 py-2 text-xs font-semibold disabled:cursor-not-allowed disabled:opacity-50"
                      data-calendar-permission-action="open_settings"
                      disabled={!onOpenCalendarSettings || calendarPermissionState === "opening"}
                      onClick={() => void performCalendarPermissionAction("open")}
                      type="button"
                    >
                      {t("chat.recovery.calendar_permission_open_settings")}
                    </button>
                  ) : null}
                  {receipt.context.availableCalendarNames.length > 0 ? (
                    <button
                      className="rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-3.5 py-2 text-xs font-semibold text-[var(--inverse-foreground)] disabled:cursor-not-allowed disabled:opacity-50"
                      disabled={!selectedCalendar || actionState === "running"}
                      data-calendar-recovery-action="use_selected"
                      onClick={() => void resolveCalendar({ resolution: "select_existing", calendarName: selectedCalendar })}
                      type="button"
                    >
                      {actionState === "running" ? t("chat.recovery.calendar_continuing") : t("chat.recovery.calendar_use_selected")}
                    </button>
                  ) : null}
                  {receipt.code === "calendar_not_found" ? (
                    <button
                      className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3.5 py-2 text-xs font-semibold disabled:cursor-not-allowed disabled:opacity-50"
                      disabled={actionState === "running"}
                      data-calendar-recovery-action="create_requested"
                      data-oomu-calendar-name={receipt.context.requestedCalendarName ?? undefined}
                      data-oomu-calendar-recovery="create-requested"
                      onClick={() => void resolveCalendar({ resolution: "create_requested" })}
                      type="button"
                    >
                      {t("chat.recovery.calendar_create", { calendar: requestedCalendar })}
                    </button>
                  ) : null}
                  <button
                    className="rounded-[var(--radius-sm)] px-3.5 py-2 text-xs font-semibold text-[var(--foreground-muted)] disabled:cursor-not-allowed disabled:opacity-50"
                    disabled={actionState === "running"}
                    data-calendar-recovery-action="cancel"
                    onClick={() => void resolveCalendar({ resolution: "cancel" })}
                    type="button"
                  >
                    {t("chat.recovery.calendar_cancel")}
                  </button>
                </div>
              </div>
            ) : actionHandler ? (
              <button
                className="rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-3.5 py-2 text-xs font-semibold text-[var(--inverse-foreground)] transition-opacity disabled:cursor-not-allowed disabled:opacity-50"
                disabled={actionState === "running"}
                onClick={() => void performAction()}
                type="button"
              >
                {actionState === "running" ? runningLabel : actionLabel}
              </button>
            ) : null}
            {!calendarResolutionSucceeded ? (
              <details
                className="group text-xs text-[var(--foreground-muted)]"
                open={receipt.recoveryAction === "review_external_changes" ? true : undefined}
              >
                <summary className="cursor-pointer font-semibold outline-none hover:text-[var(--foreground)]">
                  {t("chat.recovery.details")}
                </summary>
                <dl className="mt-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--background)] p-3">
                  <dt>{t("chat.recovery.detail_boundary")}</dt>
                  <dd className="font-medium text-[var(--foreground)]">{t(boundaryTranslationKey(receipt.boundary))}</dd>
                  <dt>{t("chat.recovery.detail_reason")}</dt>
                  <dd className="font-medium text-[var(--foreground)]">{
                    calendarRecovery || mailAutomationRecovery || remainingWorkCancellationReceipt
                      ? body
                      : receipt.message
                  }</dd>
                  {receipt.context.verifiedInputs !== null ? (
                    <><dt>{t("chat.recovery.detail_files")}</dt><dd>{receipt.context.verifiedInputs}</dd></>
                  ) : null}
                  {receipt.context.attemptCount !== null ? (
                    <><dt>{t("chat.recovery.detail_searches")}</dt><dd>{receipt.context.attemptCount}</dd></>
                  ) : null}
                  {receipt.context.pageCount !== null ? (
                    <><dt>{t("chat.recovery.detail_pages")}</dt><dd>{receipt.context.pageCount}</dd></>
                  ) : null}
                </dl>
              </details>
            ) : null}
          </div>
          {actionState === "failed"
          || calendarPermissionState === "failed"
          || mailAutomationState === "failed" ? (
            <p className="mt-3 text-xs font-medium text-[var(--destructive)]" role="alert">
              {calendarTargetRecovery
                ? t(calendarActionFailureBodyKey(calendarFailureCode), {
                    calendar: selectedCalendar || requestedCalendar,
                  })
                : t(mailAutomationRecovery
                ? "chat.recovery.mail_automation_action_failed"
                : receipt.recoveryAction === "start_new_plan"
                ? "chat.recovery.start_new_plan_failed"
                : calendarPermissionRecovery
                  ? "chat.recovery.calendar_permission_action_failed"
                  : "chat.recovery.retry_failed")}
            </p>
          ) : null}
        </div>
      </div>
    </section>
  ));
}
