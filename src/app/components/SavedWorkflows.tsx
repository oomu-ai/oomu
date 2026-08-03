"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { useAppShell } from "@/components/AppShell";
import { useI18n } from "@/context/I18nContext";
import { McpConfirmationModal } from "@/components/mcp/McpConfirmationModal";
import {
  loadSavedWorkflows,
  persistWorkflowIr,
  removeSavedWorkflow,
  type SavedWorkflow,
} from "./workflowPersistence";
import {
  friendlyWorkflowError,
  useWorkflowRun,
  workflowRunnableStepCount,
} from "./useWorkflowRun";
import { WorkflowRunFeedback } from "./WorkflowRunFeedback";

export function SavedWorkflows() {
  const { t } = useI18n();
  const {
    setWorkflowDraft,
    setWorkflowsView,
    workflowProjectScope = null,
  } = useAppShell();
  const [workflows, setWorkflows] = useState<SavedWorkflow[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [deletingWorkflowId, setDeletingWorkflowId] = useState<string | null>(null);
  const workflowRun = useWorkflowRun({
    initialStatus: t("workflows.library.loading_status"),
  });
  const { setStatus: setRunStatus } = workflowRun;
  // Deleted workflows are held briefly so Undo can restore them instead of asking for confirmation.
  const [recentlyDeleted, setRecentlyDeleted] = useState<SavedWorkflow | null>(null);
  const undoTimerRef = useRef<number | null>(null);
  const visibleWorkflows = useMemo(
    () =>
      workflowProjectScope
        ? workflows.filter(
            (workflow) =>
              workflow.projectId === workflowProjectScope.projectId,
          )
        : workflows,
    [workflowProjectScope, workflows],
  );

  useEffect(() => {
    let isCancelled = false;

    async function loadWorkflows() {
      setIsLoading(true);
      try {
        const saved = await loadSavedWorkflows();
        if (!isCancelled) {
          setWorkflows(saved);
          setRunStatus(
            saved.length === 1
              ? t("workflows.library.count_one")
              : t("workflows.library.count_many", { count: saved.length }),
          );
        }
      } catch (error) {
        if (!isCancelled) {
          setRunStatus(t("workflows.library.load_error", {
            error: friendlyWorkflowError(error, t),
          }));
        }
      } finally {
        if (!isCancelled) {
          setIsLoading(false);
        }
      }
    }

    void loadWorkflows();
    return () => {
      isCancelled = true;
    };
  }, [setRunStatus, t]);

  useEffect(() => {
    return () => {
      if (undoTimerRef.current) {
        window.clearTimeout(undoTimerRef.current);
      }
    };
  }, []);

  async function updateWorkflowLastRun(id: string, lastRunAt: number) {
    setWorkflows((current) =>
      current.map((workflow) =>
        workflow.id === id ? { ...workflow, lastRunAt } : workflow,
      ),
    );
  }

  async function deleteWorkflow(id: string) {
    const target = workflows.find((w) => w.id === id);
    if (!target) return;

    const existing = workflows;
    const updated = workflows.filter((w) => w.id !== id);
    setWorkflows(updated);
    setDeletingWorkflowId(id);
    try {
      await removeSavedWorkflow(id);
      setRunStatus(t("workflows.library.deleted_status"));
      if (undoTimerRef.current) {
        window.clearTimeout(undoTimerRef.current);
      }
      setRecentlyDeleted(target);
      undoTimerRef.current = window.setTimeout(() => setRecentlyDeleted(null), 10000);
    } catch (error) {
      setWorkflows(existing);
      setRunStatus(t("workflows.library.delete_error", {
        error: friendlyWorkflowError(error, t),
      }));
    } finally {
      setDeletingWorkflowId(null);
    }
  }

  async function undoDeleteWorkflow() {
    const target = recentlyDeleted;
    if (!target) return;

    if (undoTimerRef.current) {
      window.clearTimeout(undoTimerRef.current);
      undoTimerRef.current = null;
    }
    setRecentlyDeleted(null);
    setWorkflows((current) => [...current, target]);
    try {
      if (!target.workflowIr) {
        throw new Error(t("workflows.library.ir_required"));
      }
      await persistWorkflowIr(target, target.workflowIr);
      setRunStatus(t("workflows.library.restored_status", { name: target.name }));
    } catch (error) {
      setRunStatus(t("workflows.library.restore_error", {
        error: friendlyWorkflowError(error, t),
      }));
    }
  }

  function editWorkflow(wf: SavedWorkflow) {
    setWorkflowDraft({
      id: wf.id,
      name: wf.name,
      description: wf.description,
      workflowIr: wf.workflowIr,
      workflowVersion: wf.workflowVersion,
      compilationStatus: wf.compilationStatus,
      createdAt: wf.createdAt,
      isActive: wf.isActive,
      lastRunAt: wf.lastRunAt,
      projectId: wf.projectId ?? null,
    });
    setWorkflowsView("composer");
  }

  function createNew() {
    setWorkflowDraft(null);
    setWorkflowsView("composer");
  }

  return (
    <section className="flex h-full min-h-0 flex-col bg-[var(--background)]">
      <header className="grid shrink-0 grid-cols-1 gap-3 border-b border-[var(--border-strong)] px-5 py-4 lg:grid-cols-[minmax(0,1fr)_auto] lg:items-end">
        <div className="min-w-0">
          <h2 className="text-base font-bold text-[var(--foreground)]">
            {t("workflows.library.title")}
          </h2>
          <p className="mt-1 text-sm leading-5 text-[var(--foreground-muted)]">
            {workflowProjectScope
              ? t("workflows.scope.library_help", {
                  project: workflowProjectScope.projectName,
                })
              : t("workflows.library.description")}
          </p>
          <p
            aria-live="polite"
            className="mt-2 truncate text-xs text-[var(--foreground-subtle)]"
            title={workflowRun.status}
          >
            {workflowRun.toast ? "" : workflowRun.status}
          </p>
        </div>
        <button
          className="inline-flex shrink-0 items-center justify-center gap-1.5 justify-self-start whitespace-nowrap rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--inverse-background)] px-5 py-2.5 text-sm font-medium text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)] lg:justify-self-end"
          onClick={createNew}
          type="button"
        >
          <PlusIcon />
          {t("workflows.library.new_workflow")}
        </button>
      </header>

      <WorkflowRunFeedback feedback={workflowRun.toast} />

      <div className="min-h-0 flex-1 overflow-y-auto bg-[var(--background)] p-6">
        {isLoading ? (
          <div className="grid min-h-[24rem] place-items-center rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-8 text-center">
            <p className="text-sm text-[var(--foreground-muted)]">
              {t("workflows.library.loading")}
            </p>
          </div>
        ) : visibleWorkflows.length === 0 ? (
          <div className="grid min-h-[24rem] place-items-center rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-8 text-center">
            <div className="max-w-md">
              <p className="text-sm font-semibold text-[var(--foreground)]">
                {t("workflows.library.empty_title")}
              </p>
              <p className="mt-3 text-sm leading-6 text-[var(--foreground-muted)]">
                {workflowProjectScope
                  ? t("workflows.scope.empty_project", {
                      project: workflowProjectScope.projectName,
                    })
                  : t("workflows.library.empty_description")}
              </p>
              <button
                className="mt-6 inline-flex shrink-0 items-center justify-center gap-1.5 whitespace-nowrap rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--inverse-background)] px-4 py-2.5 text-sm font-medium text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)]"
                onClick={createNew}
                type="button"
              >
                <PlusIcon />
                {t("workflows.library.new_workflow")}
              </button>
            </div>
          </div>
        ) : (
          <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
            {visibleWorkflows.map((wf) => {
              const isRunning = workflowRun.runningWorkflowId === wf.id;
              const isWaitingForApproval = workflowRun.approvalWorkflowId === wf.id;
              const stepCount = workflowRunnableStepCount(wf);
              return (
                <div
                  className="flex flex-col justify-between rounded-[var(--radius-md)] border border-[var(--border-strong)] bg-[var(--accent-background)] p-5 transition-colors hover:bg-[var(--background)]"
                  key={wf.id}
                >
                  <div>
                    <div className="flex items-start justify-between gap-4">
                      <div className="min-w-0">
                        <h3 className="truncate text-sm font-semibold text-[var(--foreground)]">
                          {wf.name}
                        </h3>
                        <p className="mt-2 text-xs leading-5 text-[var(--foreground-muted)]">
                          {wf.description}
                        </p>
                        {!workflowProjectScope ? (
                          <p className="mt-2 text-[11px] font-semibold text-[var(--foreground-subtle)]">
                            {t(
                              wf.projectId
                                ? "workflows.scope.bound_title"
                                : "workflows.scope.global_title",
                            )}
                          </p>
                        ) : null}
                      </div>
                      {(isRunning || isWaitingForApproval) && (
                        <span className="inline-flex shrink-0 items-center gap-1 rounded-full bg-[var(--background)] px-2 py-1 text-[11px] font-medium text-[var(--accent)]">
                          {isRunning ? <SpinnerIcon /> : <ShieldIcon />}
                          {isRunning
                            ? t("workflows.library.running")
                            : t("workflows.library.approval")}
                        </span>
                      )}
                    </div>
                    <div className="mt-4 flex flex-wrap gap-x-3 gap-y-1 text-xs text-[var(--foreground-subtle)]">
                      <span>
                        {stepCount === 1
                          ? t("workflows.library.step_one")
                          : t("workflows.library.step_many", {
                              count: stepCount,
                            })}
                      </span>
                      <span>
                        {t("workflows.library.created", {
                          date: new Date(wf.createdAt).toLocaleDateString(),
                        })}
                      </span>
                      <span>{formatLastRun(wf.lastRunAt, t)}</span>
                    </div>
                  </div>
                  <div className="mt-6 grid grid-cols-[minmax(0,1fr)_auto_auto] gap-2 border-t border-[var(--border-soft)] pt-4">
                    <button
                      className="inline-flex min-h-9 items-center justify-center gap-1.5 rounded-[var(--radius-sm)] border border-transparent bg-[var(--inverse-background)] px-3 py-2 text-sm font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)] disabled:cursor-not-allowed disabled:opacity-50"
                      disabled={Boolean(workflowRun.runningWorkflowId) || Boolean(workflowRun.approvalRequest)}
                      onClick={() =>
                        void workflowRun.runWorkflow(wf, {
                          onLastRunAt: updateWorkflowLastRun,
                        })
                      }
                      type="button"
                    >
                      {isRunning ? <SpinnerIcon /> : <PlayIcon />}
                      {isRunning
                        ? t("workflows.library.running")
                        : t("workflows.library.run")}
                    </button>
                    <button
                      className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-sm font-medium text-[var(--foreground)] hover:bg-[var(--fill-hover)]"
                      onClick={() => editWorkflow(wf)}
                      type="button"
                    >
                      {t("common.edit")}
                    </button>
                    <button
                      className="rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-transparent px-3 py-2 text-sm font-medium text-[var(--destructive)] hover:bg-[var(--destructive-background)] disabled:cursor-not-allowed disabled:opacity-50"
                      disabled={deletingWorkflowId === wf.id || isRunning}
                      onClick={() => void deleteWorkflow(wf.id)}
                      type="button"
                    >
                      {t("common.delete")}
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>

      {recentlyDeleted && (
        <div className="fixed top-16 left-1/2 z-50 flex -translate-x-1/2 items-center gap-2 rounded-full border border-[var(--border-soft)] bg-[var(--background)] py-1.5 pl-4 pr-1.5 shadow-lg">
          <span className="text-sm text-[var(--foreground)]">
            {t("workflows.library.deleted", { name: recentlyDeleted.name })}
          </span>
          <button
            className="rounded-full px-3 py-1.5 text-sm font-medium text-[var(--accent)] transition-colors hover:bg-[var(--fill-hover)]"
            onClick={() => void undoDeleteWorkflow()}
            type="button"
          >
            {t("common.undo")}
          </button>
        </div>
      )}

      <McpConfirmationModal
        argumentsLabel={workflowRun.approvalPreview?.argumentsLabel}
        argumentsValue={workflowRun.approvalPreview?.argumentsValue ?? {}}
        approveLabel={
          workflowRun.isResolvingApproval
            ? t("workflows.library.approving")
            : workflowRun.approvalPreview?.reusableForWorkflowVersion
              ? t("approvals.approve_for_workflow")
              : t("workflows.library.approve")
        }
        canApprove={workflowRun.approvalPreview?.canApprove ?? false}
        isOpen={Boolean(workflowRun.approvalRequest)}
        isResolving={workflowRun.isResolvingApproval}
        onApprove={() => void workflowRun.resolveApproval("approve")}
        onCancel={() => void workflowRun.resolveApproval("reject")}
        serverLabel={workflowRun.approvalPreview?.serverLabel}
        serverName={workflowRun.approvalPreview?.serverName ?? ""}
        scopeNotice={
          workflowRun.approvalPreview?.reusableForWorkflowVersion
            ? t("approvals.reuse_scope_notice")
            : undefined
        }
        title={t("workflows.library.approve_step")}
        toolLabel={workflowRun.approvalPreview?.toolLabel}
        toolName={workflowRun.approvalPreview?.toolName ?? ""}
      />
    </section>
  );
}

function formatLastRun(lastRunAt: number | undefined, t: (key: string, variables?: Record<string, string | number>) => string) {
  if (!lastRunAt) {
    return t("workflows.library.last_run_never");
  }

  const runDate = new Date(lastRunAt);
  const now = new Date();
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const startOfRunDay = new Date(
    runDate.getFullYear(),
    runDate.getMonth(),
    runDate.getDate(),
  ).getTime();
  const dayDelta = Math.floor((startOfToday - startOfRunDay) / 86_400_000);
  const time = runDate.toLocaleTimeString([], {
    hour: "numeric",
    minute: "2-digit",
  });

  if (dayDelta === 0) {
    return t("workflows.library.last_run_today", { time });
  }

  if (dayDelta === 1) {
    return t("workflows.library.last_run_yesterday", { time });
  }

  if (dayDelta > 1 && dayDelta < 7) {
    return t("workflows.library.last_run_days", { count: dayDelta });
  }

  return t("workflows.library.last_run_date", {
    date: runDate.toLocaleDateString(),
  });
}

function PlayIcon() {
  return (
    <svg aria-hidden="true" className="h-3 w-3" fill="currentColor" viewBox="0 0 24 24">
      <path d="M8 5.5v13l11-6.5-11-6.5Z" />
    </svg>
  );
}

function PlusIcon() {
  return (
    <svg
      aria-hidden="true"
      className="h-3.5 w-3.5"
      fill="none"
      stroke="currentColor"
      strokeLinecap="round"
      strokeWidth="2"
      viewBox="0 0 24 24"
    >
      <path d="M12 5v14M5 12h14" />
    </svg>
  );
}

function SpinnerIcon() {
  return (
    <svg
      aria-hidden="true"
      className="h-3 w-3 animate-spin"
      fill="none"
      viewBox="0 0 24 24"
    >
      <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="3" />
      <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
    </svg>
  );
}

function ShieldIcon() {
  return (
    <svg
      aria-hidden="true"
      className="h-3.5 w-3.5"
      fill="none"
      stroke="currentColor"
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth="2"
      viewBox="0 0 24 24"
    >
      <path d="M12 3 4 6v6c0 4.5 3 8.5 8 10 5-1.5 8-5.5 8-10V6l-8-3Z" />
    </svg>
  );
}
