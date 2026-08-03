"use client";

import { useAppShell } from "@/components/AppShell";
import { useI18n } from "@/context/I18nContext";
import { invoke } from "@/lib/invoke";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { projectApi, type ProjectRecord } from "../projects/projectClient";
import { requestTaskFocus } from "../tasks/taskFocus";
import { RoutineDeleteDialog } from "./RoutineDeleteDialog";
import { RoutineDetails } from "./RoutineDetails";
import { RoutineHandoffNotice } from "./RoutineHandoffNotice";
import { BackgroundRuntimeCard } from "./BackgroundRuntimeCard";
import { RoutineCreateForm } from "./ScheduleBuilder";
import { useBackgroundRuntime } from "./useBackgroundRuntime";
import { useRoutineDraftHandoff } from "./useRoutineDraftHandoff";
import {
  hasChannelError,
  isChannelReady,
  type ChannelPlatform,
  type ChannelStatus,
} from "./channelReadiness";
import {
  formatRoutineTimestamp,
  humanScheduleSummary,
  routineDeleteError,
  type RoutineTranslate,
} from "./routineLabels";
import {
  routineApi,
  type RoutineHistoryItem,
  type RoutineProposal,
  type RoutineRecord,
} from "./routineClient";
import type {
  RoutineDraft,
  RoutineWorkflowAttachment,
} from "./routineDraft";
import { materializeRoutineTargetWorkflow } from "./routineTargetWorkflow";
import {
  useRoutineWorkflowHandoff,
  type RoutineWorkflowOption,
} from "./useRoutineWorkflowHandoff";
import { useRoutineProposal } from "./useRoutineProposal";

type Workflow = RoutineWorkflowOption;
type PendingRoutineDeletion = { routineId: string; label: string };
type RoutineAction =
  | ""
  | "background"
  | "create"
  | "run"
  | "pause"
  | "resume"
  | "duplicate"
  | "delivery";

export {
  backgroundStatusLabel,
  humanScheduleSummary,
  routinePausedReasonLabel,
} from "./routineLabels";

export function routineActionErrorKey(error: unknown) {
  const text =
    error instanceof Error
      ? `${error.name} ${error.message}`
      : typeof error === "string"
        ? error
        : JSON.stringify(error ?? "");
  if (text.includes("routine_workflow_project_binding_required")) {
    return "routines.error_workflow_project_required";
  }
  if (text.includes("routine_workflow_project_mismatch")) {
    return "routines.error_workflow_project_mismatch";
  }
  if (text.includes("routine_workflow_version_unavailable")) {
    return "routines.error_workflow_version_unavailable";
  }
  return "routines.error_action";
}

function cadenceKeyForRoutineDraft(draft?: RoutineDraft) {
  if (draft?.scheduleKind === "recurring") return "routines.cadence_interval";
  if (draft?.scheduleKind === "one_shot") return "routines.cadence_once";
  return "routines.cadence_daily";
}

type ReviewedRoutineCreation = {
  currentProposal: RoutineProposal;
  deliveryDestination: string;
  deliveryPlatform: ChannelPlatform | "";
  draft: RoutineDraft | null;
  label: string;
  missedPolicy: string;
  projectId: string;
  t: RoutineTranslate;
  timezone: string;
  workflowAttachment?: RoutineWorkflowAttachment;
  workflowId: string;
  workflows: Workflow[];
};

async function createReviewedRoutine({
  currentProposal,
  deliveryDestination,
  deliveryPlatform,
  draft,
  label,
  missedPolicy,
  projectId,
  t,
  timezone,
  workflowAttachment,
  workflowId,
  workflows,
}: ReviewedRoutineCreation) {
  const workflow = workflows.find((item) => item.id === workflowId);
  const attached =
    workflowAttachment && workflowId === workflowAttachment.workflowId
      ? await materializeRoutineTargetWorkflow(workflowAttachment, {
          projectDescription: t("routines.handoff_project_description"),
          projectName: t("routines.handoff_project_name"),
          workflowDescription:
            workflow?.description || t("routines.handoff_workflow_description"),
          workflowName:
            workflow?.name || t("routines.handoff_workflow_name"),
        })
      : null;
  const scheduleLabel = humanScheduleSummary(
    currentProposal.scheduleExpression,
    currentProposal.timezone,
    t,
  );
  return routineApi.create({
    confirmed: true,
    label: label || scheduleLabel,
    projectId: attached?.projectId ?? projectId,
    workflowId: attached?.workflowId ?? workflowId,
    workflowVersion:
      attached?.workflowVersion ||
      workflow?.workflowVersion ||
      workflow?.version ||
      1,
    scheduleExpression: currentProposal.scheduleExpression,
    scheduleKind: currentProposal.scheduleKind,
    timezone,
    activeWindowStartMinute: null,
    activeWindowEndMinute: null,
    endBoundary: draft?.endBoundary ?? null,
    runOnceAfterCreate: draft?.runOnceRequested ?? false,
    missedRunPolicy: missedPolicy,
    missedRunCap: 3,
    taskTemplate: {},
    modelRoute: { mode: "workflow_default" },
    deliveryTarget: deliveryPlatform
      ? { platform: deliveryPlatform, destination: deliveryDestination }
      : {},
    authority: { mode: "reviewed_workflow_scope" },
  });
}

export function RoutinesScreen({
  showIntroduction = true,
}: {
  showIntroduction?: boolean;
}) {
  const { t } = useI18n();
  const {
    routineDraft,
    setRoutineDraft,
    setActiveItem,
    setWorkflowProjectScope,
    setWorkflowsView,
  } = useAppShell();
  const [routines, setRoutines] = useState<RoutineRecord[]>([]);
  const [projects, setProjects] = useState<ProjectRecord[]>([]);
  const [workflows, setWorkflows] = useState<Workflow[]>([]);
  const {
    attachment: workflowAttachment,
    begin: beginWorkflowHandoff,
    draft: handoffDraft,
    preparationBusy: workflowPreparationBusy,
    preparationFailed: workflowPreparationFailed,
    preparationRequired: workflowPreparationRequired,
    prepare: prepareHandoffWorkflow,
    reviewProjects,
    reviewWorkflows,
  } = useRoutineWorkflowHandoff(projects, workflows, t);
  const [channelStatuses, setChannelStatuses] = useState<ChannelStatus[]>([]);
  const backgroundRuntime = useBackgroundRuntime(t);
  const [selectedId, setSelectedId] = useState("");
  const [creating, setCreating] = useState(false);
  const [scheduleText, setScheduleText] = useState("daily at 09:00");
  const [initialScheduleText, setInitialScheduleText] = useState("");
  const [scheduleCadence, setScheduleCadence] = useState(() =>
    t("routines.cadence_daily"),
  );
  const [timezone, setTimezone] = useState(
    Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC",
  );
  const {
    busy: proposalBusy,
    current: currentProposal,
    error: scheduleError,
    reset: resetProposal,
  } = useRoutineProposal(creating, scheduleText, timezone, t);
  const [label, setLabel] = useState("");
  const [labelEdited, setLabelEdited] = useState(false);
  const [projectId, setProjectId] = useState("");
  const [workflowId, setWorkflowId] = useState("");
  const [missedPolicy, setMissedPolicy] = useState("skip");
  const [deliveryPlatform, setDeliveryPlatform] = useState<
    ChannelPlatform | ""
  >("");
  const [deliveryDestination, setDeliveryDestination] = useState("");
  const [history, setHistory] = useState<RoutineHistoryItem[]>([]);
  const [historyBusy, setHistoryBusy] = useState(false);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [busyAction, setBusyAction] = useState<RoutineAction>("");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [deleteBusy, setDeleteBusy] = useState(false);
  const [deleteError, setDeleteError] = useState("");
  const [pendingDeletion, setPendingDeletion] =
    useState<PendingRoutineDeletion | null>(null);
  const deleteTriggerRef = useRef<HTMLButtonElement>(null);
  const selected =
    routines.find((item) => item.routineId === selectedId) ?? null;
  const interactionLocked =
    loading || busy || historyBusy || deleteBusy || pendingDeletion !== null;
  const connectedChannels = useMemo(
    () =>
      channelStatuses.filter(
        (status) =>
          isChannelReady(status) &&
          !hasChannelError(status) &&
          (status.platform === "discord" || Boolean(status.ownerId?.trim())),
      ),
    [channelStatuses],
  );
  const timezoneOptions = useMemo(() => {
    try {
      const zones = Intl.supportedValuesOf("timeZone");
      return zones.includes(timezone) ? zones : [timezone, ...zones];
    } catch {
      return [timezone, "UTC"];
    }
  }, [timezone]);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [
        nextRoutines,
        nextProjects,
        nextWorkflows,
        nextChannels,
      ] =
        await Promise.all([
          routineApi.list(),
          projectApi.list(),
          invoke<Workflow[]>("get_workflows"),
          invoke<ChannelStatus[]>("get_channel_statuses").catch(() => []),
        ]);
      setRoutines(nextRoutines);
      setProjects(nextProjects);
      setWorkflows(nextWorkflows);
      setChannelStatuses(Array.isArray(nextChannels) ? nextChannels : []);
      setSelectedId((current) =>
        nextRoutines.some((item) => item.routineId === current)
          ? current
          : (nextRoutines[0]?.routineId ?? ""),
      );
      setProjectId((current) =>
        current.startsWith("planned-project-") || nextProjects.some((item) => item.projectId === current)
          ? current
          : (nextProjects[0]?.projectId ?? ""),
      );
    } catch {
      setError(t("routines.error_load"));
    } finally {
      setLoading(false);
    }
  }, [t]);

  useEffect(() => {
    const timer = window.setTimeout(() => void load(), 0);
    return () => window.clearTimeout(timer);
  }, [load]);

  async function act(action: () => Promise<unknown>, actionName: RoutineAction = "") {
    setBusy(true);
    setBusyAction(actionName);
    setError("");
    setNotice("");
    setHistory([]);
    try {
      await action();
      await load();
      return true;
    } catch (cause) {
      setError(t(routineActionErrorKey(cause)));
      return false;
    } finally {
      setBusy(false);
      setBusyAction("");
    }
  }

  async function create() {
    if (!currentProposal) return;
    const created = await act(
      () =>
        createReviewedRoutine({
          currentProposal,
          deliveryDestination,
          deliveryPlatform,
          draft: handoffDraft,
          label,
          missedPolicy,
          projectId,
          t,
          timezone,
          workflowAttachment,
          workflowId,
          workflows: reviewWorkflows,
        }),
      "create",
    );
    if (!created) return;
    setCreating(false);
    beginWorkflowHandoff();
    resetProposal();
  }

  function beginCreating(initialDraft?: RoutineDraft) {
    const initialText = initialDraft?.scheduleText ?? "";
    setCreating(true);
    beginWorkflowHandoff(initialDraft);
    setInitialScheduleText(initialText);
    setScheduleText(initialText || "daily at 09:00");
    setScheduleCadence(t(cadenceKeyForRoutineDraft(initialDraft)));
    resetProposal();
    if (initialDraft?.workflowAttachment) {
      setProjectId(initialDraft.workflowAttachment.projectId);
    }
    setWorkflowId(initialDraft?.workflowAttachment?.workflowId ?? "");
    setLabel("");
    setLabelEdited(false);
    setMissedPolicy("skip");
    setDeliveryPlatform("");
    setDeliveryDestination("");
  }

  useRoutineDraftHandoff(routineDraft, setRoutineDraft, beginCreating);

  const handleScheduleChange = useCallback(
    (nextSchedule: string, cadence: string) => {
      setScheduleText(nextSchedule);
      setScheduleCadence(cadence);
      if (!labelEdited) {
        const workflow = workflows.find((item) => item.id === workflowId);
        setLabel(
          workflow
            ? t("routines.generated_name", {
                cadence,
                workflow: workflow.name,
              })
            : "",
        );
      }
    },
    [labelEdited, t, workflowId, workflows],
  );

  function chooseWorkflow(nextWorkflowId: string) {
    const workflow = workflows.find((item) => item.id === nextWorkflowId);
    setWorkflowId(nextWorkflowId);
    if (workflow?.projectId) setProjectId(workflow.projectId);
    if (!labelEdited) {
      setLabel(
        workflow
          ? t("routines.generated_name", {
              cadence: scheduleCadence,
              workflow: workflow.name,
            })
          : "",
      );
    }
  }

  function chooseProject(nextProjectId: string) {
    setProjectId(nextProjectId);
    const workflow = workflows.find((item) => item.id === workflowId);
    if (workflow && workflow.projectId !== nextProjectId) {
      setWorkflowId("");
      if (!labelEdited) setLabel("");
    }
  }

  function openProjectWorkflows(view: "composer" | "saved_workflows") {
    const project = projects.find((item) => item.projectId === projectId);
    if (!project) return;
    setActiveItem("workflows");
    setWorkflowProjectScope({
      projectId: project.projectId,
      projectName: project.name,
    });
    setWorkflowsView(view);
  }

  function chooseDelivery(nextPlatform: ChannelPlatform | "") {
    setDeliveryPlatform(nextPlatform);
    if (!nextPlatform) {
      setDeliveryDestination("");
      return;
    }
    const channel = connectedChannels.find(
      (status) => status.platform === nextPlatform,
    );
    setDeliveryDestination(
      nextPlatform === "discord" ? "" : channel?.ownerId?.trim() || "",
    );
  }

  function openTask(taskRunId: string, state: string) {
    requestTaskFocus(taskRunId, state);
    setActiveItem("tasks");
  }

  async function refreshHistory() {
    if (!selected) return;
    setHistoryBusy(true);
    setError("");
    try {
      setHistory(await routineApi.history(selected.routineId));
    } catch {
      setError(t("routines.error_history"));
    } finally {
      setHistoryBusy(false);
    }
  }

  function openDeleteConfirmation() {
    if (!selected) return;
    setDeleteError("");
    setError("");
    setNotice("");
    setPendingDeletion({
      routineId: selected.routineId,
      label: selected.label,
    });
  }

  function closeDeleteConfirmation() {
    if (deleteBusy) return;
    setDeleteError("");
    setPendingDeletion(null);
    window.setTimeout(() => deleteTriggerRef.current?.focus(), 0);
  }

  async function deleteRoutine() {
    if (!pendingDeletion) return;
    setDeleteBusy(true);
    setDeleteError("");
    setError("");
    setNotice("");
    try {
      await routineApi.remove(pendingDeletion.routineId);
      setPendingDeletion(null);
      await load();
      setNotice(t("routines.deleted"));
    } catch (cause) {
      setDeleteError(routineDeleteError(t, cause));
    } finally {
      setDeleteBusy(false);
    }
  }

  return (
    <section className="grid h-full min-h-0 grid-cols-[20rem_minmax(0,1fr)] overflow-hidden">
      <aside
        aria-hidden={pendingDeletion ? true : undefined}
        className="overflow-y-auto border-r border-[var(--border-soft)] p-5"
      >
        <div
          className={`flex items-center ${showIntroduction ? "justify-between" : "justify-end"}`}
        >
          {showIntroduction ? (
            <h1 className="text-lg font-semibold">{t("routines.title")}</h1>
          ) : null}
          <button
            className="rounded bg-[var(--inverse-background)] px-3 py-2 text-xs font-semibold text-[var(--inverse-foreground)] disabled:opacity-50"
            disabled={interactionLocked}
            onClick={() => beginCreating()}
            type="button"
          >
            {t("routines.new")}
          </button>
        </div>
        {showIntroduction ? (
          <p className="mt-1 text-sm text-[var(--foreground-muted)]">
            {t("routines.subtitle")}
          </p>
        ) : null}
        <BackgroundRuntimeCard
          busy={backgroundRuntime.busy}
          disabled={interactionLocked}
          error={backgroundRuntime.error}
          onChange={(enabled) => void backgroundRuntime.setEnabled(enabled)}
          onOpenLoginItems={() => void backgroundRuntime.openLoginItems()}
          onRefresh={() => void backgroundRuntime.refresh()}
          status={backgroundRuntime.status}
          t={t}
        />
        <div className="mt-5 grid gap-2">
          {loading && routines.length === 0 ? (
            <p
              aria-live="polite"
              className="rounded border border-dashed p-4 text-sm text-[var(--foreground-muted)]"
            >
              {t("common.loading")}
            </p>
          ) : routines.length === 0 ? (
            <p className="rounded border border-dashed p-4 text-sm text-[var(--foreground-muted)]">
              {t("routines.empty")}
            </p>
          ) : (
            routines.map((routine) => (
              <button
                className={`rounded p-3 text-left disabled:opacity-50 ${
                  selectedId === routine.routineId
                    ? "bg-[var(--fill-selected)]"
                    : "hover:bg-[var(--fill-hover)]"
                }`}
                disabled={interactionLocked}
                key={routine.routineId}
                onClick={() => {
                  setCreating(false);
                  setHistory([]);
                  setSelectedId(routine.routineId);
                }}
                type="button"
              >
                <span className="block text-sm font-semibold">
                  {routine.label}
                </span>
                <span className="mt-1 block text-xs text-[var(--foreground-muted)]">
                  {routine.deliveryState === "retrying"
                    ? t("routines.delivery_retrying_list")
                    : routine.deliveryState === "needs_review"
                      ? t("routines.delivery_review_list")
                  : routine.isActive && routine.nextRunAtMs
                    ? formatRoutineTimestamp(
                        routine.nextRunAtMs,
                        routine.timezone,
                      )
                    : t("routines.paused")}
                </span>
              </button>
            ))
          )}
        </div>
      </aside>

      <div
        aria-hidden={pendingDeletion ? true : undefined}
        className="min-h-0 overflow-y-auto p-7"
      >
        {creating ? (
          <>
            {handoffDraft ? (
              <RoutineHandoffNotice draft={handoffDraft} t={t} />
            ) : null}
            <RoutineCreateForm
              connectedChannels={connectedChannels}
              currentProposal={currentProposal}
              deliveryDestination={deliveryDestination}
              deliveryPlatform={deliveryPlatform}
              disabled={interactionLocked}
              initialScheduleText={initialScheduleText}
              isCreating={busyAction === "create"}
              label={label}
              missedPolicy={missedPolicy}
              onCreate={() => void create()}
              onDeliveryChange={chooseDelivery}
              onDeliveryDestinationChange={setDeliveryDestination}
              onLabelChange={(nextLabel) => {
                setLabelEdited(true);
                setLabel(nextLabel);
              }}
              onMissedPolicyChange={setMissedPolicy}
              onOpenConnections={() => setActiveItem("channels")}
              onOpenProjectWorkflows={openProjectWorkflows}
              onPrepareWorkflow={() => void prepareHandoffWorkflow()}
              onProjectChange={chooseProject}
              onScheduleChange={handleScheduleChange}
              onTimezoneChange={setTimezone}
              onWorkflowChange={chooseWorkflow}
              projectId={projectId}
              projects={reviewProjects}
              proposalBusy={proposalBusy}
              scheduleError={scheduleError}
              t={t}
              timezone={timezone}
              timezoneOptions={timezoneOptions}
              workflowId={workflowId}
              workflowPreparationBusy={workflowPreparationBusy}
              workflowPreparationFailed={workflowPreparationFailed}
              workflowPreparationRequired={workflowPreparationRequired}
              workflows={reviewWorkflows}
            />
          </>
        ) : selected ? (
          <RoutineDetails
            busyAction={busyAction}
            deleteTriggerRef={deleteTriggerRef}
            history={history}
            historyBusy={historyBusy}
            interactionLocked={interactionLocked}
            onDelete={openDeleteConfirmation}
            onDuplicate={() =>
              void act(
                () => routineApi.duplicate(selected.routineId),
                "duplicate",
              )
            }
            onOpenTask={openTask}
            onRefreshHistory={() => void refreshHistory()}
            onRetryDelivery={() =>
              void act(
                () => routineApi.retryDelivery(selected.routineId),
                "delivery",
              )
            }
            onRunNow={() =>
              void act(() => routineApi.runNow(selected.routineId), "run")
            }
            onToggleActive={() =>
              void act(
                () =>
                  selected.isActive
                    ? routineApi.pause(selected.routineId)
                    : routineApi.resume(selected.routineId),
                selected.isActive ? "pause" : "resume",
              )
            }
            routine={selected}
            t={t}
          />
        ) : null}
        {notice ? (
          <p aria-live="polite" className="mt-5 text-sm text-[var(--success)]">
            {notice}
          </p>
        ) : null}
        {error ? (
          <p className="mt-5 text-sm text-[var(--warning)]" role="alert">
            {error}
          </p>
        ) : null}
      </div>
      {pendingDeletion ? (
        <RoutineDeleteDialog
          busy={deleteBusy}
          error={deleteError}
          label={pendingDeletion.label}
          onCancel={closeDeleteConfirmation}
          onConfirm={() => void deleteRoutine()}
          t={t}
        />
      ) : null}
    </section>
  );
}
