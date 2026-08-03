"use client";

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  ShieldApprovalDialog,
  ShieldApprovalStatusDialog,
} from "@/app/components/chat/ShieldApprovalDialog";
import {
  resolveWorkflowPermission,
  type ApprovalRequest as WorkflowApprovalRequest,
} from "@/app/components/workflowPersistence";
import { approvalPreviewFromRequest } from "@/app/components/workflowApprovalPreview";
import { ApprovalDialogFrame } from "@/components/approvals/ApprovalDialogFrame";
import { useI18n } from "@/context/I18nContext";
import { invoke, isTauriRuntime } from "@/lib/invoke";
import type {
  ApprovalResult,
  ShieldApprovalDecision,
  ShieldApprovalDecisionOptions,
  ShieldApprovalRequest,
  ShieldApprovalStatus,
} from "@/lib/approvalContracts";

type ShieldApprovalQueueItem = {
  kind: "shield";
  request: ShieldApprovalRequest;
  sequence: number;
  source: "local" | "native";
};

type WorkflowApprovalQueueItem = {
  kind: "workflow";
  request: WorkflowApprovalRequest;
  sequence: number;
  source: "native";
};

type ApprovalQueueItem = ShieldApprovalQueueItem | WorkflowApprovalQueueItem;

type ExternalApprovalDialog = {
  id: string;
  sequence: number;
};

type ApprovalQueueState = {
  activeApprovalKey: string | null;
  items: ApprovalQueueItem[];
};

type ApprovalContextValue = {
  activeExternalDialogId: string | null;
  cancelApprovalsForSession: (sessionId: string) => Promise<void>;
  focusNextApproval: () => void;
  pendingApprovalCount: number;
  registerExternalDialog: (id: string) => () => void;
  registerWorkflowApprovalDialog: (instanceId: string) => () => void;
  requestApproval: (request: ShieldApprovalRequest) => Promise<ApprovalResult>;
  resolveWorkflowApproval: (
    instanceId: string,
    decision: "approve" | "reject",
  ) => Promise<boolean>;
  workflowApprovals: WorkflowApprovalRequest[];
};

const ApprovalContext = createContext<ApprovalContextValue | null>(null);

export function ApprovalProvider({ children }: { children: ReactNode }) {
  const [queueState, setQueueState] = useState<ApprovalQueueState>({
    activeApprovalKey: null,
    items: [],
  });
  const [externalDialogs, setExternalDialogs] = useState<ExternalApprovalDialog[]>([]);
  const [claimedWorkflowInstanceIds, setClaimedWorkflowInstanceIds] = useState<Set<string>>(
    () => new Set(),
  );
  const [dismissedApprovalKey, setDismissedApprovalKey] = useState<string | null>(null);
  const [isResolving, setIsResolving] = useState(false);
  const [resolutionError, setResolutionError] = useState<string | null>(null);
  const queueRef = useRef<ApprovalQueueItem[]>([]);
  const claimedWorkflowInstanceCountsRef = useRef(new Map<string, number>());
  const modalSequenceRef = useRef(0);
  const resolvingRef = useRef(false);
  const settlingTokensRef = useRef(new Set<string>());
  const settledTokensRef = useRef(new Set<string>());
  const localResolversRef = useRef(
    new Map<string, Set<(result: ApprovalResult) => void>>(),
  );
  const queue = queueState.items;
  const activeApprovalKey = queueState.activeApprovalKey;

  const updateQueue = useCallback(
    (updater: (current: ApprovalQueueItem[]) => ApprovalQueueItem[]) => {
      setQueueState((current) => {
        const next = updater(current.items);
        const activeApprovalKey =
          current.activeApprovalKey &&
          next.some(
            (item) =>
              approvalKey(item) === current.activeApprovalKey,
          )
            ? current.activeApprovalKey
            : next[0] ? approvalKey(next[0]) : null;
        queueRef.current = next;
        return { activeApprovalKey, items: next };
      });
    },
    [],
  );

  const enqueue = useCallback(
    (item: Omit<ShieldApprovalQueueItem, "sequence"> | Omit<WorkflowApprovalQueueItem, "sequence">) => {
      const sequence = modalSequenceRef.current++;
      updateQueue((current) => {
        const itemKey = approvalKey(item);
        if (settledTokensRef.current.has(itemKey)) {
          return current;
        }
        if (
          current.some(
            (entry) => approvalKey(entry) === itemKey,
          )
        ) {
          return current;
        }
        return [
          ...current,
          { ...item, sequence },
        ].sort(compareApprovalQueueItems);
      });
    },
    [updateQueue],
  );

  useEffect(() => {
    if (!isTauriRuntime) {
      return;
    }
    let cancelled = false;
    const unlisteners: Array<() => void> = [];
    const deferredWorkflowApprovals = new Set<number>();

    void import("@tauri-apps/api/event")
      .then(async ({ listen }) => {
        const installed = await Promise.all([
          listen<ShieldApprovalStatus>("shield-approval-status-changed", (event) => {
            if (cancelled) return;
            const request = requestFromNativeStatus(event.payload);
            if (event.payload.pending) {
              enqueue({ kind: "shield", request, source: "native" });
            } else {
              updateQueue((current) => current.filter(
                (item) => item.kind !== "shield" ||
                  item.source !== "native" ||
                  item.request.approvalToken !== event.payload.displayId,
              ));
            }
          }),
          listen<WorkflowApprovalRequest>("workflow://approval-requested", (event) => {
            const timeout = window.setTimeout(() => {
              deferredWorkflowApprovals.delete(timeout);
              if (!cancelled) {
                enqueue({ kind: "workflow", request: event.payload, source: "native" });
              }
            }, 0);
            deferredWorkflowApprovals.add(timeout);
          }),
        ]);
        if (cancelled) {
          installed.forEach((cleanup) => cleanup());
          return;
        }
        unlisteners.push(...installed);
        const [shieldRequests, workflowRequests] = await Promise.allSettled([
          invoke<ShieldApprovalStatus[]>("list_pending_shield_approvals"),
          invoke<WorkflowApprovalRequest[]>("list_pending_workflow_approvals"),
        ]);
        if (cancelled) return;
        if (shieldRequests.status === "fulfilled" && Array.isArray(shieldRequests.value)) {
          shieldRequests.value
            .filter((status) => status.pending)
            .map(requestFromNativeStatus)
            .sort(compareApprovalRequests)
            .forEach((request) =>
              enqueue({ kind: "shield", request, source: "native" }),
            );
        }
        if (workflowRequests.status === "fulfilled" && Array.isArray(workflowRequests.value)) {
          workflowRequests.value.forEach((request) =>
            enqueue({ kind: "workflow", request, source: "native" }),
          );
        }
      })
      .catch(() => {
        // Browser previews and older native shells do not expose native events.
      });

    return () => {
      cancelled = true;
      unlisteners.forEach((unlisten) => unlisten());
      deferredWorkflowApprovals.forEach((timeout) => window.clearTimeout(timeout));
    };
  }, [enqueue, updateQueue]);

  useEffect(
    () => () => {
      for (const resolvers of localResolversRef.current.values()) {
        for (const resolve of resolvers) {
          resolve({ decision: "deny", scopeKind: "once" });
        }
      }
      localResolversRef.current.clear();
    },
    [],
  );

  const requestApproval = useCallback(
    (request: ShieldApprovalRequest) =>
      new Promise<ApprovalResult>((resolve) => {
        if (settledTokensRef.current.has(`shield:${request.approvalToken}`)) {
          resolve({ decision: "deny", scopeKind: "once" });
          return;
        }
        const resolvers =
          localResolversRef.current.get(request.approvalToken) ?? new Set();
        resolvers.add(resolve);
        localResolversRef.current.set(request.approvalToken, resolvers);
        enqueue({ kind: "shield", request, source: "local" });
      }),
    [enqueue],
  );

  const settleLocal = useCallback(
    (approvalToken: string, result: ApprovalResult) => {
      const resolvers = localResolversRef.current.get(approvalToken);
      localResolversRef.current.delete(approvalToken);
      for (const resolve of resolvers ?? []) {
        resolve(result);
      }
    },
    [],
  );

  const removeApproval = useCallback(
    (item: ApprovalQueueItem) => {
      updateQueue((current) =>
        current.filter(
          (entry) => approvalKey(entry) !== approvalKey(item),
        ),
      );
      setDismissedApprovalKey((current) =>
        current === approvalKey(item) ? null : current,
      );
      setResolutionError(null);
    },
    [updateQueue],
  );

  const markSettled = useCallback((item: ApprovalQueueItem) => {
    const tokens = settledTokensRef.current;
    tokens.add(approvalKey(item));
    if (tokens.size > 2_048) {
      const oldest = tokens.values().next().value;
      if (oldest) {
        tokens.delete(oldest);
      }
    }
  }, []);

  const handleDecision = useCallback(
    async (
      itemKey: string,
      decision: ShieldApprovalDecision,
      options?: ShieldApprovalDecisionOptions,
    ) => {
      const item = queueRef.current.find((entry) => approvalKey(entry) === itemKey);
      if (
        !item ||
        (item.kind === "shield" && item.source === "native") ||
        resolvingRef.current ||
        settlingTokensRef.current.has(itemKey)
      ) {
        return false;
      }
      const scopeKind = options?.trustScopeKind ?? "once";
      settlingTokensRef.current.add(itemKey);
      resolvingRef.current = true;
      setIsResolving(true);
      setResolutionError(null);
      try {
        if (item.kind === "workflow") {
          await resolveWorkflowPermission(
            item.request,
            decision === "approve" ? "approve" : "reject",
          );
        } else {
          settleLocal(item.request.approvalToken, { decision, scopeKind });
        }
        markSettled(item);
        removeApproval(item);
        return true;
      } catch (error) {
        if (isTerminalApprovalError(error)) {
          markSettled(item);
          removeApproval(item);
          return true;
        } else {
          setResolutionError(item.kind === "workflow" ? "approvals.resolve_failed" : "permissions.resolve_failed");
          return false;
        }
      } finally {
        settlingTokensRef.current.delete(itemKey);
        resolvingRef.current = false;
        setIsResolving(false);
      }
    },
    [markSettled, removeApproval, settleLocal],
  );

  const cancelApprovalsForSession = useCallback(
    async (sessionId: string) => {
      const matching = queueRef.current.filter(
        (item) =>
          item.kind === "shield" &&
          item.request.sessionId === sessionId &&
          !settlingTokensRef.current.has(approvalKey(item)),
      );
      if (!matching.length) return;
      matching.forEach((item) => {
        settlingTokensRef.current.add(approvalKey(item));
        markSettled(item);
        if (item.source === "local") {
          settleLocal(item.request.approvalToken, {
            decision: "deny",
            scopeKind: "once",
          });
        }
      });
      updateQueue((current) =>
        current.filter((item) => item.kind !== "shield" || item.request.sessionId !== sessionId),
      );
      await Promise.allSettled(matching.map(async (item) => {
        if (item.source === "local") {
          await invoke<void>("mcp_reject_tool_approval", {
            approvalToken: item.request.approvalToken,
          });
        }
      }));
      matching.forEach((item) => {
        settlingTokensRef.current.delete(approvalKey(item));
      });
    },
    [markSettled, settleLocal, updateQueue],
  );

  const registerExternalDialog = useCallback((id: string) => {
    const normalizedId = id.trim();
    if (!normalizedId) {
      return () => undefined;
    }
    const sequence = modalSequenceRef.current++;
    setExternalDialogs((current) => {
      if (current.some((dialog) => dialog.id === normalizedId)) {
        return current;
      }
      return [
        ...current,
        {
          id: normalizedId,
          sequence,
        },
      ].sort(compareExternalDialogs);
    });
    return () => {
      setExternalDialogs((current) =>
        current.filter((dialog) => dialog.id !== normalizedId),
      );
    };
  }, []);

  const refreshWorkflowApprovals = useCallback(async () => {
    try {
      const requests = await invoke<WorkflowApprovalRequest[]>(
        "list_pending_workflow_approvals",
      );
      const pendingKeys = new Set(
        requests.map((request) => `workflow:${request.approvalToken}`),
      );
      updateQueue((current) => current.filter(
        (item) => item.kind !== "workflow" || pendingKeys.has(approvalKey(item)),
      ));
      requests.forEach((request) =>
        enqueue({ kind: "workflow", request, source: "native" }),
      );
    } catch {
      // Native recovery is best-effort while a live workflow surface changes hands.
    }
  }, [enqueue, updateQueue]);

  const registerWorkflowApprovalDialog = useCallback((instanceId: string) => {
    const normalizedId = instanceId.trim();
    if (!normalizedId) return () => undefined;
    const counts = claimedWorkflowInstanceCountsRef.current;
    counts.set(normalizedId, (counts.get(normalizedId) ?? 0) + 1);
    setClaimedWorkflowInstanceIds(new Set(counts.keys()));
    return () => {
      const remaining = (counts.get(normalizedId) ?? 1) - 1;
      if (remaining > 0) {
        counts.set(normalizedId, remaining);
      } else {
        counts.delete(normalizedId);
        void refreshWorkflowApprovals();
      }
      setClaimedWorkflowInstanceIds(new Set(counts.keys()));
    };
  }, [refreshWorkflowApprovals]);

  const eligibleQueue = useMemo(
    () => queue.filter(
      (item) => item.kind !== "workflow" ||
        !claimedWorkflowInstanceIds.has(item.request.instanceId),
    ),
    [claimedWorkflowInstanceIds, queue],
  );
  const pendingApprovalCount = eligibleQueue.length;

  const nextApproval =
    eligibleQueue.find((item) => approvalKey(item) === activeApprovalKey) ??
    eligibleQueue[0] ??
    null;
  const nextExternalDialog = externalDialogs[0] ?? null;
  const earliestApprovalSequence = eligibleQueue.reduce(
    (earliest, item) => Math.min(earliest, item.sequence),
    Number.MAX_SAFE_INTEGER,
  );
  const externalHasTurn = Boolean(
    nextExternalDialog &&
      (!nextApproval ||
        nextExternalDialog.sequence < earliestApprovalSequence),
  );
  const activeExternalDialogId = externalHasTurn
    ? nextExternalDialog?.id ?? null
    : null;

  const focusNextApproval = useCallback(() => {
    setDismissedApprovalKey(null);
  }, []);
  const workflowApprovals = useMemo(
    () => queue.flatMap((item) => item.kind === "workflow" ? [item.request] : []),
    [queue],
  );
  const resolveWorkflowApproval = useCallback(
    async (instanceId: string, decision: "approve" | "reject") => {
      const item = queueRef.current.find(
        (entry) => entry.kind === "workflow" && entry.request.instanceId === instanceId,
      );
      return item
        ? handleDecision(approvalKey(item), decision === "approve" ? "approve" : "deny")
        : false;
    },
    [handleDecision],
  );

  const value: ApprovalContextValue = {
    activeExternalDialogId,
    cancelApprovalsForSession,
    focusNextApproval,
    pendingApprovalCount,
    registerExternalDialog,
    registerWorkflowApprovalDialog,
    requestApproval,
    resolveWorkflowApproval,
    workflowApprovals,
  };
  const current = externalHasTurn ||
    (nextApproval && dismissedApprovalKey === approvalKey(nextApproval))
    ? null
    : nextApproval;

  return (
    <ApprovalContext.Provider value={value}>
      {children}
      {current?.kind === "shield" && current.source === "local" ? (
        <ShieldApprovalDialog
          isResolving={isResolving}
          key={current.request.approvalToken}
          onApprove={(options) => void handleDecision(approvalKey(current), "approve", options)}
          onDeny={() => void handleDecision(approvalKey(current), "deny")}
          request={current.request}
          resolutionErrorKey={resolutionError}
        />
      ) : null}
      {current?.kind === "shield" && current.source === "native" ? (
        <ShieldApprovalStatusDialog
          key={current.request.approvalToken}
          onDismiss={() => setDismissedApprovalKey(approvalKey(current))}
          request={current.request}
        />
      ) : null}
      {current?.kind === "workflow" ? (
        <WorkflowApprovalDialog
          isResolving={isResolving}
          key={current.request.approvalToken}
          onApprove={() => void handleDecision(approvalKey(current), "approve")}
          onDecline={() => void handleDecision(approvalKey(current), "deny")}
          onDismiss={() => setDismissedApprovalKey(approvalKey(current))}
          request={current.request}
          resolutionErrorKey={resolutionError}
        />
      ) : null}
    </ApprovalContext.Provider>
  );
}

export function useApproval() {
  const value = useContext(ApprovalContext);
  if (!value) {
    throw new Error("useApproval must be used within ApprovalProvider.");
  }
  return value;
}

export function useOptionalApproval() {
  return useContext(ApprovalContext);
}

export function useApprovalDialogTurn(isRequested: boolean, dialogId: string) {
  const context = useContext(ApprovalContext);
  const registerExternalDialog = context?.registerExternalDialog;

  useEffect(() => {
    if (!isRequested || !registerExternalDialog) {
      return;
    }
    return registerExternalDialog(dialogId);
  }, [dialogId, isRequested, registerExternalDialog]);

  if (!isRequested) {
    return false;
  }
  return context ? context.activeExternalDialogId === dialogId : true;
}

export function useWorkflowApprovalClaim(instanceId: string | null | undefined) {
  const context = useContext(ApprovalContext);
  const registerWorkflowApprovalDialog = context?.registerWorkflowApprovalDialog;

  useLayoutEffect(() => {
    if (!instanceId || !registerWorkflowApprovalDialog) return;
    return registerWorkflowApprovalDialog(instanceId);
  }, [instanceId, registerWorkflowApprovalDialog]);
}

function WorkflowApprovalDialog({
  isResolving,
  onApprove,
  onDecline,
  onDismiss,
  request,
  resolutionErrorKey,
}: {
  isResolving: boolean;
  onApprove: () => void;
  onDecline: () => void;
  onDismiss: () => void;
  request: WorkflowApprovalRequest;
  resolutionErrorKey: string | null;
}) {
  const { t } = useI18n();
  const preview = approvalPreviewFromRequest(request, t);
  return (
    <ApprovalDialogFrame
      description={t("approvals.description")}
      eyebrow={t("approvals.paused")}
      footer={<>
        <button className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-4 py-2 text-sm font-semibold transition-colors hover:bg-[var(--fill-hover)] disabled:cursor-wait disabled:opacity-50" data-approval-initial-focus disabled={isResolving} onClick={onDecline} type="button">
          {t("approvals.decline")}
        </button>
        <button aria-busy={isResolving} className="rounded-[var(--radius-sm)] bg-[var(--accent)] px-4 py-2 text-sm font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)] disabled:cursor-wait disabled:opacity-50" disabled={isResolving || !preview.canApprove} onClick={onApprove} type="button">
          {isResolving
            ? t("approvals.resolving")
            : preview.reusableForWorkflowVersion
              ? t("approvals.approve_for_workflow")
              : t("approvals.approve")}
        </button>
      </>}
      onDismiss={() => { if (!isResolving) onDismiss(); }}
      title={t("approvals.title")}
    >
      <dl className="mt-5 grid gap-3 rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-4 text-xs sm:grid-cols-2">
        <div><dt className="text-[var(--foreground-muted)]">{preview.toolLabel}</dt><dd className="mt-1 font-semibold text-[var(--foreground)]">{preview.toolName}</dd></div>
        <div><dt className="text-[var(--foreground-muted)]">{preview.serverLabel}</dt><dd className="mt-1 font-semibold text-[var(--foreground)]">{preview.serverName}</dd></div>
      </dl>
      {preview.argumentsValue.length > 0 ? (
        <section
          aria-label={preview.argumentsLabel}
          className="mt-4 rounded-[var(--radius-md)] border border-[var(--border-soft)] p-4"
        >
          <h3 className="text-sm font-semibold">{preview.argumentsLabel}</h3>
          <ul className="mt-2 grid gap-2 text-sm">
            {preview.argumentsValue.map((value) => (
              <li
                className="break-words rounded-[var(--radius-sm)] bg-[var(--accent-background)] px-3 py-2"
                key={value}
              >
                {value}
              </li>
            ))}
          </ul>
        </section>
      ) : null}
      {preview.reusableForWorkflowVersion ? (
        <p className="mt-4 rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-3 text-sm text-[var(--foreground-muted)]">
          {t("approvals.reuse_scope_notice")}
        </p>
      ) : null}
      {!preview.canApprove ? <p className="mt-4 rounded-[var(--radius-sm)] bg-[var(--warning-background)] p-3 text-sm font-medium text-[var(--warning)]" role="alert">{t("permissions.unverified_action")}</p> : null}
      {resolutionErrorKey ? <p className="mt-4 rounded-[var(--radius-sm)] bg-[var(--destructive-background)] p-3 text-sm text-[var(--destructive)]" role="alert">{t(resolutionErrorKey)}</p> : null}
      <details className="mt-5 rounded-[var(--radius-md)] border border-[var(--border-soft)]">
        <summary className="cursor-pointer px-3 py-2.5 text-sm font-semibold">{t("common.details")}</summary>
        <dl className="grid gap-3 border-t border-[var(--border-soft)] bg-[var(--accent-background)] p-3 text-xs sm:grid-cols-2">
          <div><dt className="text-[var(--foreground-subtle)]">{t("approvals.run")}</dt><dd className="mt-1 font-medium">{safeWorkflowIdentifier(request.instanceId) || t("common.unknown")}</dd></div>
          <div><dt className="text-[var(--foreground-subtle)]">{t("approvals.step")}</dt><dd className="mt-1 font-medium">{safeWorkflowIdentifier(request.nodeId) || t("common.unknown")}</dd></div>
        </dl>
      </details>
    </ApprovalDialogFrame>
  );
}

function safeWorkflowIdentifier(value: string) {
  const normalized = value.replace(/[^A-Za-z0-9 _.-]/g, " ").replace(/\s+/g, " ").trim();
  return normalized.slice(0, 96);
}

function requestFromNativeStatus(status: ShieldApprovalStatus): ShieldApprovalRequest {
  return {
    approvalToken: status.displayId,
    sessionId: status.sessionId,
    actionType: "native_shield_status",
    actionLabel: status.actionLabel,
    riskTier: "native",
    reason: status.semanticSummary,
    requestedAtMs: status.requestedAtMs,
    preview: "",
    semanticSummary: status.semanticSummary,
    approvalScopeKinds: ["once"],
  };
}

function approvalToken(item: Pick<ApprovalQueueItem, "request">) {
  return item.request.approvalToken;
}

function approvalKey(item: Pick<ApprovalQueueItem, "kind" | "request">) {
  return `${item.kind}:${approvalToken(item)}`;
}

function normalizedRequestedAt(value: number) {
  return Number.isFinite(value) && value > 0 ? value : Number.MAX_SAFE_INTEGER;
}

function compareApprovalQueueItems(
  left: ApprovalQueueItem,
  right: ApprovalQueueItem,
) {
  if (left.kind === "workflow" || right.kind === "workflow") {
    return left.sequence - right.sequence;
  }
  return (
    normalizedRequestedAt(left.request.requestedAtMs) -
      normalizedRequestedAt(right.request.requestedAtMs) ||
    left.sequence - right.sequence
  );
}

function compareApprovalRequests(
  left: ShieldApprovalRequest,
  right: ShieldApprovalRequest,
) {
  return normalizedRequestedAt(left.requestedAtMs) -
    normalizedRequestedAt(right.requestedAtMs);
}

function compareExternalDialogs(
  left: ExternalApprovalDialog,
  right: ExternalApprovalDialog,
) {
  return left.sequence - right.sequence;
}

const TERMINAL_APPROVAL_ERROR_CODES = new Set([
  "shield_approval_not_found",
  "shield_approval_channel_closed",
  "shield_approval_timeout",
  "shield_approval_resume_failed",
  "workflow_runtime_approval_consumed",
  "workflow_runtime_approval_state_invalid",
]);

function isTerminalApprovalError(error: unknown) {
  if (typeof error === "string") {
    try {
      return isTerminalApprovalError(JSON.parse(error));
    } catch {
      return false;
    }
  }
  const code = error &&
    typeof error === "object" &&
    "code" in error &&
    typeof error.code === "string"
    ? error.code
    : "";
  return TERMINAL_APPROVAL_ERROR_CODES.has(code);
}
