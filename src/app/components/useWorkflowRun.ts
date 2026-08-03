"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useWorkflowApprovalClaim } from "@/context/ApprovalContext";
import { useI18n } from "@/context/I18nContext";
import { approvalPreviewFromRequest } from "./workflowApprovalPreview";
import {
  executeCompiledWorkflow,
  markWorkflowLastRun,
  resolveWorkflowPermission,
  type ApprovalRequest,
  type SavedWorkflow,
  type WorkflowPreflightMode,
  type WorkflowRunResponse,
} from "./workflowPersistence";

export type WorkflowRunToast = {
  message: string;
  tone: "success" | "error" | "info";
};

export type WorkflowRunProgress = {
  nodeId: string;
  stepIndex: number;
  status: string;
  message: string;
};

type WorkflowRunOptions = {
  input?: unknown;
  inputNodeId?: string;
  onLastRunAt?: (workflowId: string, lastRunAt: number) => void | Promise<void>;
  preflightMode?: WorkflowPreflightMode;
  workflowVersion?: number;
};

type ApprovalContext = {
  options: WorkflowRunOptions;
  workflow: SavedWorkflow;
};

export function useWorkflowRun({
  initialStatus,
}: {
  initialStatus?: string;
} = {}) {
  const { t } = useI18n();
  const [runningWorkflowId, setRunningWorkflowId] = useState<string | null>(null);
  const [approvalWorkflowId, setApprovalWorkflowId] = useState<string | null>(null);
  const [approvalRequest, setApprovalRequest] = useState<ApprovalRequest | null>(null);
  const [isResolvingApproval, setIsResolvingApproval] = useState(false);
  const [status, setStatus] = useState(initialStatus ?? "");
  const [toast, setToast] = useState<WorkflowRunToast | null>(null);
  const [lastRun, setLastRun] = useState<WorkflowRunResponse | null>(null);
  const [lastRunDurationMs, setLastRunDurationMs] = useState<number | null>(null);
  const [progress, setProgress] = useState<WorkflowRunProgress | null>(null);
  const runStartedAtRef = useRef<number | null>(null);
  const approvalContextRef = useRef<ApprovalContext | null>(null);
  const approvalResolutionRef = useRef(false);
  const isRunningRef = useRef(false);

  useWorkflowApprovalClaim(approvalRequest?.instanceId);

  const approvalPreview = useMemo(
    () => (approvalRequest ? approvalPreviewFromRequest(approvalRequest, t) : null),
    [approvalRequest, t],
  );

  useEffect(() => {
    if (!toast || toast.tone === "error") {
      return;
    }
    const timeout = window.setTimeout(() => setToast(null), 5000);
    return () => window.clearTimeout(timeout);
  }, [toast]);

  const showToast = useCallback(
    (tone: WorkflowRunToast["tone"], message: string) => {
      setToast({ message, tone });
    },
    [],
  );

  // The runtime emits one `vwa://progress` event per node transition
  // (running → success/bypassed/halted). Surface the latest so the composer can show a live
  // indicator instead of a frozen Run button. Browser dev has no Tauri event bridge,
  // so the dynamic import simply no-ops there.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        const off = await listen<{
          block_id?: unknown;
          step_index?: unknown;
          status?: unknown;
          message?: unknown;
        }>("vwa://progress", (event) => {
          if (disposed || !isRunningRef.current) {
            return;
          }
          const payload = event.payload;
          setProgress({
            nodeId: typeof payload.block_id === "string" ? payload.block_id : "",
            stepIndex:
              typeof payload.step_index === "number" ? payload.step_index : 0,
            status: typeof payload.status === "string" ? payload.status : "running",
            message: typeof payload.message === "string" ? payload.message : "",
          });
        });
        if (disposed) {
          off();
          return;
        }
        unlisten = off;
      } catch {
        // No event bridge outside the desktop runtime; the indicator stays idle.
      }
    })();
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const elapsedRunMs = useCallback(() => {
    return runStartedAtRef.current === null
      ? undefined
      : new Date().getTime() - runStartedAtRef.current;
  }, []);

  const recordLastRun = useCallback(
    async (
      workflow: SavedWorkflow,
      lastRunAt: number,
      options: WorkflowRunOptions,
    ) => {
      await options.onLastRunAt?.(workflow.id, lastRunAt);
      try {
        const didUpdate = await markWorkflowLastRun(workflow.id, lastRunAt);
        if (!didUpdate) {
          throw new Error("The saved workflow row was not found.");
        }
      } catch (error) {
        showToast(
          "error",
          t("workflows.library.last_run_save_error", {
            message: friendlyWorkflowError(error, t),
          }),
        );
      }
    },
    [showToast, t],
  );

  const handleRunResponse = useCallback(
    async (
      workflow: SavedWorkflow,
      response: WorkflowRunResponse,
      options: WorkflowRunOptions,
      elapsedMs?: number,
    ) => {
      setLastRun(response);
      setLastRunDurationMs(resolveWorkflowDurationMs(response, elapsedMs));

      if (response.instance.status === "Completed") {
        runStartedAtRef.current = null;
        approvalContextRef.current = null;
        const completedAt = new Date().getTime();
        await recordLastRun(workflow, completedAt, options);
        const completedEmpty =
          response.completion?.kind === "empty_collection";
        setStatus(
          t(
            completedEmpty
              ? "workflows.library.completed_empty_status"
              : "workflows.library.completed_status",
            { name: workflow.name },
          ),
        );
        showToast(
          "success",
          t(
            completedEmpty
              ? "workflows.library.completed_empty_toast"
              : "workflows.library.completed_toast",
            { name: workflow.name },
          ),
        );
        return;
      }

      if (response.instance.status === "AwaitingApproval" && response.approvalRequest) {
        runStartedAtRef.current = null;
        approvalContextRef.current = { workflow, options };
        setApprovalWorkflowId(workflow.id);
        setApprovalRequest(response.approvalRequest);
        setLastRunDurationMs(null);
        setStatus(t("workflows.library.approval_status", { name: workflow.name }));
        showToast(
          "info",
          t("workflows.library.approval_toast", { name: workflow.name }),
        );
        return;
      }

      approvalContextRef.current = null;
      setApprovalWorkflowId(null);
      setApprovalRequest(null);

      if (response.instance.status === "Failed") {
        runStartedAtRef.current = null;
        const message = resolveFailedRunMessage(response.instance, t);
        setStatus(message);
        showToast(
          "error",
          t("workflows.library.failed_toast", { name: workflow.name, message }),
        );
        return;
      }

      setStatus(
        t("workflows.library.returned_status", {
          name: workflow.name,
          status: response.instance.status,
        }),
      );
      showToast(
        "info",
        t("workflows.library.returned_status", {
          name: workflow.name,
          status: response.instance.status,
        }),
      );
    },
    [recordLastRun, showToast, t],
  );

  const runWorkflow = useCallback(
    async (workflow: SavedWorkflow, options: WorkflowRunOptions = {}) => {
      if (workflowRunnableStepCount(workflow) === 0) {
        showToast(
          "error",
          t("workflows.library.no_steps", { name: workflow.name }),
        );
        return null;
      }

      const inputNodeId = options.inputNodeId ?? workflowInputNodeId(workflow);
      const workflowVersion =
        options.workflowVersion ??
        workflow.workflowVersion ??
        workflow.workflowIr?.workflowVersion;
      const input =
        options.input ??
        {
          description: workflow.description,
          objective:
            workflow.name.trim() || t("workflows.library.default_objective"),
        };

      setRunningWorkflowId(workflow.id);
      setApprovalWorkflowId(null);
      setApprovalRequest(null);
      setLastRun(null);
      setLastRunDurationMs(null);
      setProgress(null);
      setStatus(t("workflows.library.running_status", { name: workflow.name }));
      setToast(null);
      runStartedAtRef.current = new Date().getTime();
      isRunningRef.current = true;

      try {
        const response = await executeCompiledWorkflow(
          workflow.id,
          workflowVersion,
          inputNodeId,
          input,
          options.preflightMode ?? "skipped",
        );
        await handleRunResponse(workflow, response, options, elapsedRunMs());
        return response;
      } catch (error) {
        runStartedAtRef.current = null;
        const message = friendlyWorkflowError(error, t);
        setStatus(message);
        showToast(
          "error",
          t("workflows.library.could_not_run", { name: workflow.name, message }),
        );
        return null;
      } finally {
        isRunningRef.current = false;
        setProgress(null);
        setRunningWorkflowId(null);
      }
    },
    [elapsedRunMs, handleRunResponse, showToast, t],
  );

  const resolveApproval = useCallback(
    async (decision: "approve" | "reject") => {
      if (
        approvalResolutionRef.current ||
        !approvalRequest ||
        !approvalContextRef.current
      ) {
        return;
      }

      const { workflow, options } = approvalContextRef.current;
      approvalResolutionRef.current = true;
      setIsResolvingApproval(true);
      setRunningWorkflowId(workflow.id);
      setProgress(null);
      setStatus(
        decision === "approve"
          ? t("workflows.library.continuing", { name: workflow.name })
          : t("workflows.library.stopping", { name: workflow.name }),
      );
      runStartedAtRef.current = new Date().getTime();
      isRunningRef.current = true;

      try {
        const response = await resolveWorkflowPermission(approvalRequest, decision);
        setApprovalRequest(null);
        setApprovalWorkflowId(null);
        await handleRunResponse(workflow, response, options, elapsedRunMs());
      } catch (error) {
        runStartedAtRef.current = null;
        const message = friendlyWorkflowError(error, t);
        setStatus(message);
        showToast(
          "error",
          t("workflows.library.could_not_continue", {
            name: workflow.name,
            message,
          }),
        );
      } finally {
        approvalResolutionRef.current = false;
        isRunningRef.current = false;
        setProgress(null);
        setIsResolvingApproval(false);
        setRunningWorkflowId(null);
      }
    },
    [approvalRequest, elapsedRunMs, handleRunResponse, showToast, t],
  );

  return {
    approvalPreview,
    approvalRequest,
    approvalWorkflowId,
    isResolvingApproval,
    lastRun,
    lastRunDurationMs,
    progress,
    resolveApproval,
    runWorkflow,
    runningWorkflowId,
    setStatus,
    setToast,
    showToast,
    status,
    toast,
  };
}

export function workflowRunnableStepCount(workflow: SavedWorkflow) {
  return (
    workflow.workflowIr.nodes.filter(
      (node) => node.kind !== "input" && node.kind !== "output",
    ).length
  );
}

export function workflowInputNodeId(workflow: SavedWorkflow) {
  return (
    workflow.workflowIr.nodes.find((node) => node.kind === "input")?.id ??
    `${workflow.id}:input`
  );
}

export function resolveFailedRunMessage(
  instance: WorkflowRunResponse["instance"],
  t?: (key: string, variables?: Record<string, string | number>) => string,
) {
  const failedPayload = Object.values(instance.nodePayloads).find(
    (payload) => payload.status === "Failed" && payload.error != null,
  );
  // Prefer the node-specific error (it names the failing step and boundary), then fall
  // back to the instance-level error the runtime sets for structural failures that never
  // reach a node payload (e.g. "No reachable Output node completed.", invalid edges).
  // Without the instance.error fallback those collapse to the opaque "Unknown execution
  // error." the user reported.
  return friendlyWorkflowError(
    failedPayload?.error ?? instance.error ?? instance.outputPayload,
    t,
  );
}

export function friendlyWorkflowError(
  error: unknown,
  t?: (key: string, variables?: Record<string, string | number>) => string,
) {
  const fallback = t?.("workflows.library.unknown_error") ?? "Unknown execution error.";
  const validationMessage =
    t?.("workflows.library.validation_error") ??
    "This workflow needs a valid storyboard before it can run. Review the steps, save again, and try once more.";
  const saveMessage =
    t?.("workflows.library.persistence_error") ??
    "The workflow ran into a local save or database problem. Save it again, then run it once more.";
  const localAppMessage =
    t?.("workflows.library.local_app_error") ??
    "OOMU couldn't reach the Apple app this workflow needs. Try again.";
  const calendarTimeoutMessage =
    t?.("workflows.library.calendar_timeout_error") ??
    "Calendar took too long to respond. Try again.";
  const calendarPermissionMessage =
    t?.("workflows.library.calendar_permission_error") ??
    "Calendar access needs to be refreshed. Open System Settings, turn OOMU's Calendar access off and back on, then try again.";
  const calendarUnavailableMessage =
    t?.("workflows.library.calendar_unavailable_error") ??
    "Calendar couldn't be read right now. Try again.";
  const stepTimeoutMessage =
    t?.("workflows.library.step_timeout_error") ??
    "This step took too long to respond. Try again.";
  const notificationUnavailableMessage =
    t?.("workflows.library.notification_unavailable_error") ??
    "Notifications are off for OOMU. Turn them on in System Settings, then try again.";
  const stepDataMessage =
    t?.("workflows.library.step_data_error") ??
    "This workflow couldn't use the result from an earlier step. Nothing was changed. Try again.";

  if (typeof error === "string" && error.trim()) {
    return normalizeWorkflowErrorMessage(
      error,
      validationMessage,
      saveMessage,
      localAppMessage,
      calendarTimeoutMessage,
      calendarPermissionMessage,
      calendarUnavailableMessage,
      stepTimeoutMessage,
      notificationUnavailableMessage,
      stepDataMessage,
    );
  }

  if (
    error &&
    typeof error === "object" &&
    "message" in error &&
    typeof error.message === "string" &&
    error.message.trim()
  ) {
    return normalizeWorkflowErrorMessage(
      error.message,
      validationMessage,
      saveMessage,
      localAppMessage,
      calendarTimeoutMessage,
      calendarPermissionMessage,
      calendarUnavailableMessage,
      stepTimeoutMessage,
      notificationUnavailableMessage,
      stepDataMessage,
      "code" in error && typeof error.code === "string" ? error.code : undefined,
    );
  }

  return fallback;
}

function normalizeWorkflowErrorMessage(
  message: string,
  validationMessage: string,
  persistenceMessage: string,
  localAppMessage: string,
  calendarTimeoutMessage: string,
  calendarPermissionMessage: string,
  calendarUnavailableMessage: string,
  stepTimeoutMessage: string,
  notificationUnavailableMessage: string,
  stepDataMessage: string,
  code?: string,
) {
  const normalized = message.toLowerCase();
  if (code === "workflow_runtime_notification_unavailable") {
    return notificationUnavailableMessage;
  }
  if (code === "workflow_runtime_calendar_permission") {
    return calendarPermissionMessage;
  }
  if (
    code === "workflow_runtime_calendar_unavailable" ||
    code === "workflow_runtime_calendar_not_found"
  ) {
    return calendarUnavailableMessage;
  }
  if (code === "workflow_runtime_calendar_timeout") {
    return calendarTimeoutMessage;
  }
  if (
    code === "workflow_runtime_node_timeout" ||
    normalized.includes("node execution timed out")
  ) {
    return normalized.includes("calendar")
      ? calendarTimeoutMessage
      : stepTimeoutMessage;
  }
  if (normalized.includes("template reference") && normalized.includes("unresolved")) {
    return stepDataMessage;
  }
  if (
    normalized.includes("macos_applescript") ||
    (normalized.includes("apple app") && normalized.includes("offline")) ||
    normalized.includes("non-native transport")
  ) {
    return localAppMessage;
  }
  if (
    code === "workflow_runtime_ir_invalid" ||
    normalized.includes("zoderror") ||
    normalized.includes("workflow_ir_invalid") ||
    normalized.includes("workflow_runtime_ir_invalid") ||
    normalized.includes("workflow ir")
  ) {
    return validationMessage;
  }

  if (
    normalized.includes("rusqlite") ||
    normalized.includes("sqlcipher") ||
    normalized.includes("database") ||
    normalized.includes("saved workflow row was not found")
  ) {
    return persistenceMessage;
  }

  return message.trim();
}

function resolveWorkflowDurationMs(
  response: WorkflowRunResponse,
  elapsedMs?: number,
) {
  const payloadDurations = Object.values(response.instance.nodePayloads)
    .map((payload) => payload.latencyMs)
    .filter((duration): duration is number => typeof duration === "number");

  if (payloadDurations.length > 0) {
    return payloadDurations.reduce((total, duration) => total + duration, 0);
  }

  return elapsedMs ?? null;
}
