"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { useOptionalApproval } from "@/context/ApprovalContext";
import { useI18n } from "@/context/I18nContext";
import type { TaskState } from "@/lib/p0Contracts";
import type { P0EventEnvelope } from "@/lib/p0Contracts";
import { projectApi, type ProjectRecord } from "../projects/projectClient";
import { approvalPreviewFromRequest } from "../workflowApprovalPreview";
import { taskApi, type TaskRun } from "./taskClient";
import {
  taskEffectVerificationFromEvents,
  type TaskEffectVerification,
  type TaskEffectVerificationDecision,
} from "./taskEffectVerification";
import { TaskEffectVerificationCard } from "./TaskEffectVerificationCard";
import { BrowserTaskPanel } from "../browser_automation/BrowserTaskPanel";
import { ChildWorkstreams } from "../delegation/ChildWorkstreams";
import { EvidenceTimeline } from "./EvidenceTimeline";
import { CreateDocumentAction } from "../artifacts/review/CreateDocumentAction";
import { MediaTaskPanel } from "../media/MediaTaskPanel";
import { LearningReview } from "../learning/LearningReview";
import { AnalysisResults } from "../analysis/AnalysisResults";
import { consumeTaskFocus, peekTaskFocus } from "./taskFocus";
import { ScreenEmptyState } from "../shared/ScreenEmptyState";

const attentionFilters: TaskState[] = [
  "running",
  "awaiting_approval",
  "blocked",
];
const historyFilters: TaskState[] = ["completed", "failed", "cancelled"];

type TranslateFn = (key: string, values?: Record<string, string | number>) => string;
type ApprovalPreview = ReturnType<typeof approvalPreviewFromRequest>;

const originLabelKeys: Record<string, string> = {
  taskflow: "tasks.origin_task",
  workflow: "tasks.origin_workflow",
  agent: "tasks.origin_agent",
  chat_queue: "tasks.origin_chat",
  routine: "tasks.origin_scheduled",
};

export function taskOriginLabel(t: TranslateFn, value: string) {
  return t(originLabelKeys[value] ?? "tasks.origin_other");
}

export function taskErrorLabel(
  t: TranslateFn,
  lastError: string | null,
  recoveryState: string,
) {
  if (!lastError) return null;

  const normalized = lastError.trim().toLowerCase();
  if (recoveryState === "lost" || normalized.includes("runtime record is missing")) {
    return t("tasks.error_record_missing");
  }
  if (recoveryState === "runtime_unavailable" || normalized.includes("owning runtime")) {
    return t("tasks.error_runtime_unavailable");
  }
  if (recoveryState === "recoverable") {
    return t("tasks.error_recoverable");
  }
  if (/official (page|source)|http 40[0-9]/.test(normalized)) {
    return t("tasks.error_official_source");
  }
  if (/credential|sign.?in|authoriz|permission|access denied|forbidden/.test(normalized)) {
    return t("tasks.error_access");
  }
  if (/network|offline|timed? out|timeout|connection|service unavailable/.test(normalized)) {
    return t("tasks.error_connection");
  }
  if (/cancelled|canceled/.test(normalized)) {
    return t("tasks.error_cancelled");
  }
  return t("tasks.error_generic");
}

export function TaskErrorSummary({
  lastError,
  recoveryState,
  t,
}: {
  lastError: string | null;
  recoveryState: string;
  t: TranslateFn;
}) {
  const label = taskErrorLabel(t, lastError, recoveryState);
  if (!label) return null;

  return (
    <div>
      <h3 className="text-sm font-semibold">{t("tasks.error_title")}</h3>
      <p className="mt-2 rounded-[var(--radius-sm)] bg-[var(--accent-background)] p-3 text-sm">
        {label}
      </p>
    </div>
  );
}

export function TaskCenter({
  onStartInChat,
  showIntroduction = true,
}: {
  onStartInChat?: () => void;
  showIntroduction?: boolean;
}) {
  const { t } = useI18n();
  const approvals = useOptionalApproval();
  const [initialTaskNavigation] = useState(() => {
    const focus = peekTaskFocus();
    return {
      filter: focus?.state ?? (focus ? "all" : "running"),
      historyOpen: Boolean(
        focus && (!focus.state || historyFilters.includes(focus.state)),
      ),
    } as const;
  });
  const [tasks, setTasks] = useState<TaskRun[]>([]);
  const [projects, setProjects] = useState<ProjectRecord[]>([]);
  const [projectId, setProjectId] = useState("");
  const [filter, setFilter] = useState<TaskState | "all">(
    initialTaskNavigation.filter,
  );
  const [historyOpen, setHistoryOpen] = useState(initialTaskNavigation.historyOpen);
  const [selectedId, setSelectedId] = useState("");
  const [events, setEvents] = useState<P0EventEnvelope[]>([]);
  const [eventsState, setEventsState] = useState<"loading" | "ready" | "error">("loading");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  const [notice, setNotice] = useState("");
  const [controlling, setControlling] = useState<
    TaskRun["validControls"][number] | ""
  >("");
  const [approvalDecision, setApprovalDecision] = useState<"approve" | "reject" | "">("");
  const [effectDecision, setEffectDecision] = useState<
    TaskEffectVerificationDecision | ""
  >("");
  const loadSequence = useRef(0);
  const eventLoadSequence = useRef(0);

  const load = useCallback(async ({ silent = false }: { silent?: boolean } = {}) => {
    const sequence = ++loadSequence.current;
    if (!silent) setLoading(true);
    try {
      const records = await taskApi.list(projectId || undefined, filter === "all" ? undefined : filter);
      if (sequence !== loadSequence.current) return;
      setTasks(records);
      const requestedTask = consumeTaskFocus();
      setSelectedId((current) => requestedTask && records.some((task) => task.taskRunId === requestedTask.taskRunId)
        ? requestedTask.taskRunId
        : records.some((task) => task.taskRunId === current) ? current : records[0]?.taskRunId ?? "");
      setError("");
    } catch {
      if (sequence === loadSequence.current) setError(t("tasks.load_error"));
    } finally {
      if (!silent && sequence === loadSequence.current) setLoading(false);
    }
  }, [filter, projectId, t]);

  useEffect(() => { void projectApi.list().then(setProjects).catch(() => setProjects([])); }, []);
  useEffect(() => {
    let cancelled = false;

    async function refresh(silent: boolean) {
      try {
        await taskApi.reconcile();
      } catch {
        // Background reconciliation is intentionally silent. Row recovery and
        // error states remain the source of truth when attention is required.
      }
      if (!cancelled) await load({ silent });
    }

    const timeout = window.setTimeout(() => void refresh(false), 0);
    const interval = window.setInterval(() => void refresh(true), 5_000);
    const handleFocus = () => void refresh(true);
    window.addEventListener("focus", handleFocus);

    return () => {
      cancelled = true;
      loadSequence.current += 1;
      window.clearTimeout(timeout);
      window.clearInterval(interval);
      window.removeEventListener("focus", handleFocus);
    };
  }, [load]);
  const selected = tasks.find((task) => task.taskRunId === selectedId) ?? null;
  const selectedWorkflowApproval = selected?.runtimeKind === "workflow"
    ? approvals?.workflowApprovals.find(
      (approval) => approval.instanceId === selected.runtimeRecordId,
    ) ?? null
    : null;
  const selectedWorkflowApprovalPreview = selectedWorkflowApproval
    ? approvalPreviewFromRequest(selectedWorkflowApproval, t)
    : null;
  const selectedEffectVerification = selected
    ? taskEffectVerificationFromEvents(selected, events)
    : null;
  const selectedEventRevision = selected?.updatedAtMs ?? 0;
  const selectedEventRevisionRef = useRef(selectedEventRevision);
  useEffect(() => {
    selectedEventRevisionRef.current = selectedEventRevision;
  }, [selectedEventRevision]);
  const loadSelectedEvents = useCallback(async () => {
    const sequence = ++eventLoadSequence.current;
    const revision = selectedEventRevision;
    if (!selectedId) {
      setEvents([]);
      setEventsState("ready");
      return;
    }

    setEventsState("loading");
    try {
      const records = await taskApi.events(selectedId);
      if (
        sequence !== eventLoadSequence.current ||
        revision !== selectedEventRevisionRef.current
      ) return;
      setEvents(records);
      setEventsState("ready");
    } catch {
      if (
        sequence !== eventLoadSequence.current ||
        revision !== selectedEventRevisionRef.current
      ) return;
      setEvents([]);
      setEventsState("error");
    }
  }, [selectedEventRevision, selectedId]);
  useEffect(() => {
    const timeout = window.setTimeout(() => void loadSelectedEvents(), 0);
    return () => {
      window.clearTimeout(timeout);
      eventLoadSequence.current += 1;
    };
  }, [loadSelectedEvents]);

  async function control(controlName: TaskRun["validControls"][number]) {
    if (!selected) return;
    const command = controlName === "cancel" ? "cancel_task_run" : controlName === "resume" ? "resume_task_run" : controlName === "retry" ? "retry_task_run" : "acknowledge_task_failure";
    setControlling(controlName);
    setNotice("");
    setError("");
    try {
      if (controlName === "resume" && selected.runtimeKind === "agent") {
        await taskApi.resumeAgent(selected.runtimeRecordId);
      } else {
        await taskApi.control(command, selected.taskRunId);
      }
      await load();
      setNotice(t("tasks.control_success"));
    } catch {
      setError(t("tasks.control_error"));
    } finally {
      setControlling("");
    }
  }

  async function resolveApproval(decision: "approve" | "reject") {
    if (!selected || !approvals || !selectedWorkflowApproval) return;
    setApprovalDecision(decision);
    setNotice("");
    setError("");
    try {
      const resolved = await approvals.resolveWorkflowApproval(
        selectedWorkflowApproval.instanceId,
        decision,
      );
      if (!resolved) {
        setError(t("tasks.approval_error"));
        return;
      }
      await taskApi.reconcile().catch(() => undefined);
      await load();
      setNotice(t("tasks.approval_resolved"));
    } catch {
      setError(t("tasks.approval_error"));
    } finally {
      setApprovalDecision("");
    }
  }

  async function resolveEffectVerification(
    decision: TaskEffectVerificationDecision,
  ) {
    if (
      !selected ||
      (!selectedEffectVerification && decision !== "stop_without_repeating")
    ) return;
    setEffectDecision(decision);
    setNotice("");
    setError("");
    try {
      const baseRequest = {
        runtimeRecordId: selected.runtimeRecordId,
        taskId: selected.taskId,
        taskRunId: selected.taskRunId,
      };
      if (decision === "stop_without_repeating") {
        await taskApi.resolveEffectVerification(selectedEffectVerification
          ? {
              ...baseRequest,
              decision,
              effectKind: selectedEffectVerification.effectKind,
              idempotencyKey: selectedEffectVerification.idempotencyKey,
              nodeId: selectedEffectVerification.nodeId,
              verificationSequence:
                selectedEffectVerification.verificationSequence,
            }
          : { ...baseRequest, decision });
      } else if (selectedEffectVerification) {
        await taskApi.resolveEffectVerification({
          ...baseRequest,
          decision,
          effectKind: selectedEffectVerification.effectKind,
          idempotencyKey: selectedEffectVerification.idempotencyKey,
          nodeId: selectedEffectVerification.nodeId,
          verificationSequence: selectedEffectVerification.verificationSequence,
        });
      }
      await load();
      setNotice(
        t(
          decision === "did_not_happen"
            ? "tasks.effect_verification_retry_saved"
            : "tasks.effect_verification_stop_saved",
        ),
      );
    } catch {
      setError(t("tasks.effect_verification_error"));
    } finally {
      setEffectDecision("");
    }
  }

  function toggleHistory() {
    if (historyOpen) {
      setHistoryOpen(false);
      if (
        filter === "all" ||
        filter === "completed" ||
        filter === "failed" ||
        filter === "cancelled"
      ) {
        setFilter("running");
      }
      return;
    }
    setHistoryOpen(true);
  }

  return (
    <section className="flex h-full min-h-0 flex-col overflow-hidden p-6">
      <div className="flex flex-wrap items-start justify-between gap-4">
        {showIntroduction ? (
          <div>
            <h1 className="text-2xl font-semibold">{t("tasks.title")}</h1>
            <p className="mt-1 text-sm text-[var(--foreground-muted)]">
              {t("tasks.subtitle")}
            </p>
          </div>
        ) : (
          <div>
            <p className="text-sm font-medium">{t("tasks.now_title")}</p>
            <p className="mt-1 text-sm text-[var(--foreground-muted)]">
              {t("tasks.now_help")}
            </p>
          </div>
        )}
        <select
          aria-label={t("tasks.project_filter")}
          className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-sm"
          disabled={loading || Boolean(controlling)}
          onChange={(event) => setProjectId(event.target.value)}
          value={projectId}
        >
          <option value="">{t("tasks.all_projects")}</option>
          {projects.map((project) => (
            <option key={project.projectId} value={project.projectId}>
              {project.name}
            </option>
          ))}
        </select>
      </div>
      <div className="mt-5 flex items-center gap-1 overflow-x-auto">
        {attentionFilters.map((state) => (
          <button
            aria-pressed={filter === state}
            className={`rounded-full px-3 py-1.5 text-xs font-semibold transition-colors ${
              filter === state
                ? "bg-[var(--inverse-background)] text-[var(--inverse-foreground)]"
                : "bg-[var(--accent-background)] text-[var(--foreground-muted)] hover:bg-[var(--fill-hover)] hover:text-[var(--foreground)]"
            }`}
            key={state}
            onClick={() => setFilter(state)}
            type="button"
          >
            {t(`tasks.filter_${state}`)}
          </button>
        ))}
        <div className="ml-auto flex items-center gap-1 border-l border-[var(--border-soft)] pl-3">
          <button
            aria-controls="task-history-filters"
            aria-expanded={historyOpen}
            className={`rounded-full px-3 py-1.5 text-xs font-semibold transition-colors ${
              historyOpen
                ? "bg-[var(--fill-selected)] text-[var(--foreground)]"
                : "text-[var(--foreground-muted)] hover:bg-[var(--fill-hover)] hover:text-[var(--foreground)]"
            }`}
            onClick={toggleHistory}
            type="button"
          >
            {t("tasks.filter_history")}
          </button>
          {historyOpen ? (
            <div className="flex items-center gap-1" id="task-history-filters">
              {["all" as const, ...historyFilters].map((state) => (
                <button
                  aria-pressed={filter === state}
                  className={`rounded-full px-3 py-1.5 text-xs font-semibold transition-colors ${
                    filter === state
                      ? "bg-[var(--inverse-background)] text-[var(--inverse-foreground)]"
                      : "bg-[var(--accent-background)] text-[var(--foreground-muted)] hover:bg-[var(--fill-hover)] hover:text-[var(--foreground)]"
                  }`}
                  key={state}
                  onClick={() => setFilter(state)}
                  type="button"
                >
                  {t(`tasks.filter_${state}`)}
                </button>
              ))}
            </div>
          ) : null}
        </div>
      </div>
      {notice ? (
        <p
          aria-live="polite"
          className="mt-3 rounded-[var(--radius-sm)] border border-[var(--success)]/30 bg-[var(--success-background)] px-3 py-2 text-sm text-[var(--success)]"
          role="status"
        >
          {notice}
        </p>
      ) : null}
      {error ? (
        <p
          className="mt-3 rounded-[var(--radius-sm)] border border-[var(--destructive)]/30 bg-[var(--destructive-background)] px-3 py-2 text-sm text-[var(--destructive)]"
          role="alert"
        >
          {error}
        </p>
      ) : null}
      <div className="mt-5 grid min-h-0 flex-1 grid-cols-[19rem_minmax(0,1fr)] overflow-hidden rounded-[var(--radius-md)] border border-[var(--border-soft)]">
        {loading ? (
          <p className="col-span-2 self-center justify-self-center p-5 text-sm text-[var(--foreground-muted)]">
            {t("common.loading")}
          </p>
        ) : tasks.length === 0 ? (
          <ScreenEmptyState
            actionLabel={onStartInChat ? t("tasks.go_to_chat") : undefined}
            body={t("tasks.empty")}
            className="col-span-2 m-6 self-center"
            icon={<ChatEmptyIcon />}
            onAction={onStartInChat}
            title={t("tasks.empty_title")}
          />
        ) : (
          <>
            <div className="min-h-0 overflow-y-auto border-r border-[var(--border-soft)]">
              {tasks.map((task) => (
                <button
                  className={`block w-full border-b border-[var(--border-soft)] p-4 text-left transition-colors ${
                    selectedId === task.taskRunId
                      ? "bg-[var(--fill-selected)]"
                      : "hover:bg-[var(--fill-hover)]"
                  }`}
                  key={task.taskRunId}
                  onClick={() => setSelectedId(task.taskRunId)}
                  type="button"
                >
                  <span className="flex items-center justify-between gap-2">
                    <span className="truncate text-sm font-semibold">
                      {task.summary || t("tasks.untitled")}
                    </span>
                    <span className="rounded-full bg-[var(--accent-background)] px-2 py-1 text-[10px]">
                      {t(`tasks.state_${task.state}`)}
                    </span>
                  </span>
                  <span className="mt-2 block text-xs text-[var(--foreground-muted)]">
                    {taskOriginLabel(t, task.origin)} {" · "}
                    {new Date(task.updatedAtMs).toLocaleString()}
                  </span>
                </button>
              ))}
            </div>
            <div className="min-h-0 overflow-y-auto p-6">
              {selected ? (
                <TaskDetail
                  approvalDecision={approvalDecision}
                  approvalPreview={selectedWorkflowApprovalPreview}
                  busyControl={controlling}
                  effectDecision={effectDecision}
                  effectDetailsState={eventsState}
                  effectVerification={selectedEffectVerification}
                  events={events}
                  hasWorkflowApproval={Boolean(selectedWorkflowApproval)}
                  onApprovalDecision={(decision) => void resolveApproval(decision)}
                  onControl={control}
                  onEffectDecision={(decision) => void resolveEffectVerification(decision)}
                  onReloadEffectDetails={() => void loadSelectedEvents()}
                  task={selected}
                  t={t}
                />
              ) : (
                <p className="text-sm text-[var(--foreground-muted)]">
                  {t("tasks.select")}
                </p>
              )}
            </div>
          </>
        )}
      </div>
    </section>
  );
}

function ChatEmptyIcon() {
  return (
    <span className="flex h-11 w-11 items-center justify-center rounded-full bg-[var(--accent-background)] text-[var(--foreground-muted)]">
      <svg
        aria-hidden="true"
        className="h-5 w-5"
        fill="none"
        stroke="currentColor"
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth="1.8"
        viewBox="0 0 24 24"
      >
        <path d="M4 5h16v11H8l-4 4V5Z" />
        <path d="M8 9h8M8 12h5" />
      </svg>
    </span>
  );
}

function TaskDetail({
  approvalDecision,
  approvalPreview,
  busyControl,
  effectDecision,
  effectDetailsState,
  effectVerification,
  events,
  hasWorkflowApproval,
  onApprovalDecision,
  onControl,
  onEffectDecision,
  onReloadEffectDetails,
  task,
  t,
}: {
  approvalDecision: "approve" | "reject" | "";
  approvalPreview: ApprovalPreview | null;
  busyControl: TaskRun["validControls"][number] | "";
  effectDecision: TaskEffectVerificationDecision | "";
  effectDetailsState: "loading" | "ready" | "error";
  effectVerification: TaskEffectVerification | null;
  events: P0EventEnvelope[];
  hasWorkflowApproval: boolean;
  onApprovalDecision: (decision: "approve" | "reject") => void;
  onControl: (control: TaskRun["validControls"][number]) => void;
  onEffectDecision: (decision: TaskEffectVerificationDecision) => void;
  onReloadEffectDetails: () => void;
  task: TaskRun;
  t: TranslateFn;
}) {
  const needsWorkflowApproval = task.state === "awaiting_approval" && task.runtimeKind === "workflow";
  const approvalBusy = Boolean(approvalDecision);
  const secondaryClassName = needsWorkflowApproval
    ? "space-y-6 border-t border-[var(--border-soft)] pt-5 opacity-80"
    : "space-y-6";

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h2 className="text-xl font-semibold">{task.summary || t("tasks.untitled")}</h2>
        <p className="mt-2 text-sm text-[var(--foreground-muted)]">
          {t("tasks.detail_meta", {
            state: t(`tasks.state_${task.state}`),
            origin: taskOriginLabel(t, task.origin),
          })}
        </p>
      </div>
      {needsWorkflowApproval ? (
        <section className="rounded-[var(--radius-md)] border border-[var(--warning)]/30 bg-[var(--warning-background)] p-5">
          <h3 className="text-base font-semibold">{t("tasks.approval_title")}</h3>
          <p className="mt-1 text-sm text-[var(--foreground-muted)]">
            {hasWorkflowApproval
              ? approvalPreview?.canApprove
                ? t("tasks.approval_help")
                : t("permissions.unverified_action")
              : t("tasks.approval_recovering")}
          </p>
          {approvalPreview ? (
            <div className="mt-4 rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--background)] p-4">
              <dl className="space-y-3 text-sm">
                <div>
                  <dt className="text-xs font-medium text-[var(--foreground-muted)]">
                    {approvalPreview.toolLabel}
                  </dt>
                  <dd className="mt-1 font-semibold">{approvalPreview.toolName}</dd>
                </div>
                {approvalPreview.argumentsValue.length > 0 ? (
                  <div>
                    <dt className="text-xs font-medium text-[var(--foreground-muted)]">
                      {approvalPreview.argumentsLabel}
                    </dt>
                    <dd className="mt-1 space-y-1">
                      {approvalPreview.argumentsValue.map((value) => (
                        <span className="block" key={value}>{value}</span>
                      ))}
                    </dd>
                  </div>
                ) : null}
              </dl>
            </div>
          ) : null}
          <div className="mt-4 flex flex-wrap gap-2">
            <button
              aria-busy={approvalDecision === "reject"}
              className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-4 py-2 text-sm font-semibold transition-colors hover:bg-[var(--fill-hover)] disabled:cursor-wait disabled:opacity-50"
              disabled={approvalBusy || !hasWorkflowApproval}
              onClick={() => onApprovalDecision("reject")}
              type="button"
            >
              {t("approvals.decline")}
            </button>
            <button
              aria-busy={approvalDecision === "approve"}
              className="rounded-[var(--radius-sm)] bg-[var(--accent)] px-4 py-2 text-sm font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)] disabled:cursor-wait disabled:opacity-50"
              disabled={approvalBusy || !hasWorkflowApproval || !approvalPreview?.canApprove}
              onClick={() => onApprovalDecision("approve")}
              type="button"
            >
              {t("approvals.approve")}
            </button>
          </div>
        </section>
      ) : null}
      {task.effectVerificationRequired ? (
        <TaskEffectVerificationCard
          decision={effectDecision}
          detailsState={effectDetailsState}
          onDecision={onEffectDecision}
          onReload={onReloadEffectDetails}
          t={t}
          verification={effectVerification}
        />
      ) : task.recoveryState !== "not_required" && task.recoveryState !== "reconciled" ? (
        <div className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--warning-background)] p-4">
          <p className="text-sm font-semibold">{t("tasks.recovery_title")}</p>
          <p className="mt-1 text-sm">{t(`tasks.recovery_${task.recoveryState}`)}</p>
        </div>
      ) : null}
      {!task.effectVerificationRequired ? (
        <TaskErrorSummary lastError={task.lastError} recoveryState={task.recoveryState} t={t} />
      ) : null}
      <div className="flex flex-wrap gap-2">
        {task.validControls.map((control) => (
          <button
            aria-busy={busyControl === control}
            className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-sm font-semibold transition-colors hover:bg-[var(--fill-hover)] disabled:cursor-not-allowed disabled:opacity-50"
            data-action-state={busyControl === control ? "working" : "idle"}
            disabled={Boolean(busyControl)}
            key={control}
            onClick={() => onControl(control)}
            type="button"
          >
            {busyControl === control ? t("tasks.control_working") : t(`tasks.control_${control}`)}
          </button>
        ))}
      </div>
      <div className={secondaryClassName}>
        {needsWorkflowApproval ? <h3 className="text-xs font-semibold uppercase tracking-wide text-[var(--foreground-subtle)]">{t("tasks.supporting_details")}</h3> : null}
        {task.projectId ? <MediaTaskPanel projectId={task.projectId} taskId={task.taskId} taskRunId={task.taskRunId} /> : null}
        <CreateDocumentAction events={events} task={task} />
        <AnalysisResults taskRunId={task.taskRunId} />
        <ChildWorkstreams taskRunId={task.taskRunId} />
        {task.projectId ? <LearningReview completed={task.state === "completed"} projectId={task.projectId} taskRunId={task.taskRunId} /> : null}
        {task.projectId ? <BrowserTaskPanel projectId={task.projectId} t={t} taskRunId={task.taskRunId} /> : null}
        <div>
          <h3 className="text-sm font-semibold">{t("tasks.activity")}</h3>
          <EvidenceTimeline emptyLabel={t("tasks.no_events")} events={events} />
        </div>
      </div>
    </div>
  );
}
