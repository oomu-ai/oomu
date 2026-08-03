"use client";

import { useRef } from "react";
import type { ProjectRecord } from "../projects/projectClient";
import type { ChannelPlatform, ChannelStatus } from "./channelReadiness";
import {
  formatRoutineTimestamp,
  humanScheduleSummary,
  humanTimezoneLabel,
  type RoutineTranslate,
} from "./routineLabels";
import type { RoutineProposal } from "./routineClient";
import { RoutineAuthorityReview } from "./RoutineAuthorityReview";
import { ScheduleBuilder } from "./ScheduleCadenceFields";
import {
  type WorkflowReviewCapabilities,
  unavailableWorkflowReview,
  workflowReviewCapabilities,
} from "./workflowReviewCapabilities";
export { ScheduleBuilder } from "./ScheduleCadenceFields";
export { composeScheduleText } from "./routineScheduleCadence";

type WorkflowOption = {
  id: string;
  name: string;
  projectId?: string | null;
  reviewCapabilities?: WorkflowReviewCapabilities;
  steps?: string;
  description?: string;
};

function deliveryDestinationCopy(platform: ChannelPlatform | "") {
  if (platform === "slack") {
    return {
      label: "routines.slack_conversation",
      placeholder: "routines.slack_conversation_placeholder",
      hint: "routines.slack_conversation_hint",
    };
  }
  if (platform === "discord") {
    return {
      label: "routines.discord_channel",
      placeholder: "routines.discord_channel_placeholder",
      hint: "routines.discord_channel_hint",
    };
  }
  return null;
}

function WorkflowPreparationCard({
  busy,
  disabled,
  failed,
  onRetry,
  required,
  t,
  visible,
}: {
  busy: boolean;
  disabled: boolean;
  failed: boolean;
  onRetry: () => void;
  required: boolean;
  t: RoutineTranslate;
  visible: boolean;
}) {
  if (!visible || !required) return null;
  return (
    <div
      className="mt-3 rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-4"
      role="status"
    >
      <p className="text-sm font-semibold">
        {t(
          failed
            ? "routines.handoff_prepare_failed_title"
            : "routines.handoff_prepare_title",
        )}
      </p>
      <p className="mt-1 text-xs leading-5 text-[var(--foreground-muted)]">
        {t(
          failed
            ? "routines.handoff_prepare_failed_help"
            : "routines.handoff_prepare_help",
        )}
      </p>
      {failed ? (
        <button
          className="mt-3 rounded-[var(--radius-sm)] border border-[var(--border-strong)] px-3 py-2 text-sm font-semibold disabled:opacity-50"
          disabled={disabled || busy}
          onClick={onRetry}
          type="button"
        >
          {t("routines.handoff_prepare_retry")}
        </button>
      ) : null}
    </div>
  );
}

function RoutineConfirmAction({
  currentProposal,
  deliveryDestination,
  deliveryHint,
  deliveryPlatform,
  disabled,
  isCreating,
  onCreate,
  projectId,
  proposalBusy,
  reviewReady,
  t,
  workflowId,
  workflowMatchesProject,
  workflowPreparationRequired,
}: {
  currentProposal: RoutineProposal | null;
  deliveryDestination: string;
  deliveryHint: string;
  deliveryPlatform: ChannelPlatform | "";
  disabled: boolean;
  isCreating: boolean;
  onCreate: () => void;
  projectId: string;
  proposalBusy: boolean;
  reviewReady: boolean;
  t: RoutineTranslate;
  workflowId: string;
  workflowMatchesProject: boolean;
  workflowPreparationRequired: boolean;
}) {
  const deliveryIncomplete = Boolean(deliveryPlatform) && !deliveryDestination;
  const blocked =
    disabled ||
    proposalBusy ||
    !currentProposal ||
    !projectId ||
    !workflowId ||
    !workflowMatchesProject ||
    workflowPreparationRequired ||
    !reviewReady ||
    deliveryIncomplete;
  const hint = !workflowId
    ? "routines.choose_workflow_hint"
    : !projectId
      ? "routines.choose_project_hint"
      : !workflowMatchesProject
        ? "routines.workflow_project_mismatch_hint"
        : workflowPreparationRequired
          ? "routines.handoff_prepare_hint"
          : !reviewReady
            ? "routines.authority_unavailable_hint"
            : deliveryIncomplete
              ? deliveryHint
              : !currentProposal
                ? "routines.schedule_hint"
                : "";
  return (
    <div className="mt-7 flex flex-wrap items-center gap-3">
      <button
        className="w-fit rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-4 py-2 text-sm font-semibold text-[var(--inverse-foreground)] disabled:opacity-50"
        disabled={blocked}
        onClick={onCreate}
        type="button"
      >
        {isCreating ? t("routines.creating") : t("routines.confirm")}
      </button>
      {hint ? (
        <span className="text-xs text-[var(--foreground-muted)]">
          {t(hint)}
        </span>
      ) : null}
    </div>
  );
}

export function RoutineCreateForm({
  connectedChannels,
  currentProposal,
  deliveryDestination,
  deliveryPlatform,
  disabled,
  initialScheduleText,
  isCreating,
  label,
  missedPolicy,
  onCreate,
  onDeliveryChange,
  onDeliveryDestinationChange,
  onLabelChange,
  onMissedPolicyChange,
  onOpenConnections,
  onOpenProjectWorkflows,
  onPrepareWorkflow = () => undefined,
  onProjectChange,
  onScheduleChange,
  onTimezoneChange,
  onWorkflowChange,
  projectId,
  projects,
  proposalBusy,
  scheduleError,
  t,
  timezone,
  timezoneOptions,
  workflowId,
  workflowPreparationBusy = false,
  workflowPreparationFailed = false,
  workflowPreparationRequired = false,
  workflows,
}: {
  connectedChannels: ChannelStatus[];
  currentProposal: RoutineProposal | null;
  deliveryDestination: string;
  deliveryPlatform: ChannelPlatform | "";
  disabled: boolean;
  initialScheduleText?: string;
  isCreating: boolean;
  label: string;
  missedPolicy: string;
  onCreate: () => void;
  onDeliveryChange: (platform: ChannelPlatform | "") => void;
  onDeliveryDestinationChange: (destination: string) => void;
  onLabelChange: (label: string) => void;
  onMissedPolicyChange: (policy: string) => void;
  onOpenConnections: () => void;
  onOpenProjectWorkflows: (view: "composer" | "saved_workflows") => void;
  onPrepareWorkflow?: () => void;
  onProjectChange: (projectId: string) => void;
  onScheduleChange: (schedule: string, cadence: string) => void;
  onTimezoneChange: (timezone: string) => void;
  onWorkflowChange: (workflowId: string) => void;
  projectId: string;
  projects: ProjectRecord[];
  proposalBusy: boolean;
  scheduleError: string;
  t: RoutineTranslate;
  timezone: string;
  timezoneOptions: string[];
  workflowId: string;
  workflowPreparationBusy?: boolean;
  workflowPreparationFailed?: boolean;
  workflowPreparationRequired?: boolean;
  workflows: WorkflowOption[];
}) {
  const advancedRef = useRef<HTMLDetailsElement>(null);
  const destinationCopy = deliveryDestinationCopy(deliveryPlatform);
  const selectedWorkflow =
    workflows.find((item) => item.id === workflowId) ?? null;
  const selectedProject =
    projects.find((item) => item.projectId === projectId) ?? null;
  const compatibleWorkflows = selectedProject
    ? workflows.filter(
        (workflow) => workflow.projectId === selectedProject.projectId,
      )
    : [];
  const workflowMatchesProject = Boolean(
    selectedWorkflow &&
      selectedProject &&
      selectedWorkflow.projectId === selectedProject.projectId,
  );
  const reviewCapabilities = selectedWorkflow
    ? (selectedWorkflow.reviewCapabilities ??
      workflowReviewCapabilities(selectedWorkflow.steps))
    : unavailableWorkflowReview;
  function openAdvanced() {
    if (!advancedRef.current) return;
    advancedRef.current.open = true;
    window.setTimeout(
      () =>
        advancedRef.current
          ?.querySelector<HTMLSelectElement>("select")
          ?.focus(),
      0,
    );
  }

  return (
    <div className="mx-auto max-w-2xl">
      <h2 className="text-2xl font-semibold">{t("routines.create_title")}</h2>
      <label className="mt-6 grid gap-2 text-sm font-semibold">
        {t("routines.project")}
        <select
          className="rounded-[var(--radius-sm)] border bg-[var(--background)] px-3 py-2 font-normal"
          disabled={disabled}
          onChange={(event) => onProjectChange(event.target.value)}
          value={projectId}
        >
          <option value="">{t("routines.choose_project")}</option>
          {projects.map((project) => (
            <option key={project.projectId} value={project.projectId}>
              {project.name}
            </option>
          ))}
        </select>
        <span className="text-xs font-normal text-[var(--foreground-muted)]">
          {t("routines.project_scope_help")}
        </span>
      </label>
      <label className="mt-6 grid gap-2 text-sm font-semibold">
        {t("routines.workflow")}
        <select
          className="rounded-[var(--radius-sm)] border bg-[var(--background)] px-3 py-2 font-normal"
          disabled={disabled}
          onChange={(event) => onWorkflowChange(event.target.value)}
          value={workflowId}
        >
          <option value="">{t("routines.choose_workflow")}</option>
          {compatibleWorkflows.map((workflow) => (
            <option key={workflow.id} value={workflow.id}>
              {workflow.name}
            </option>
          ))}
        </select>
        {selectedWorkflow?.description ? (
          <span className="text-xs font-normal text-[var(--foreground-muted)]">
            {selectedWorkflow.description}
          </span>
        ) : null}
      </label>
      <WorkflowPreparationCard
        busy={workflowPreparationBusy}
        disabled={disabled}
        failed={workflowPreparationFailed}
        onRetry={onPrepareWorkflow}
        required={workflowPreparationRequired}
        t={t}
        visible={Boolean(selectedWorkflow)}
      />
      {selectedProject && compatibleWorkflows.length === 0 ? (
        <div
          className="mt-3 rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-4"
          role="status"
        >
          <p className="text-sm font-semibold">
            {t("routines.no_project_workflows_title", {
              project: selectedProject.name,
            })}
          </p>
          <p className="mt-1 text-xs leading-5 text-[var(--foreground-muted)]">
            {t("routines.no_project_workflows_help")}
          </p>
          <button
            className="mt-3 rounded-[var(--radius-sm)] border border-[var(--border-strong)] px-3 py-2 text-sm font-semibold"
            disabled={disabled}
            onClick={() => onOpenProjectWorkflows("composer")}
            type="button"
          >
            {t("routines.create_project_workflow")}
          </button>
        </div>
      ) : null}
      {selectedWorkflow && !workflowMatchesProject ? (
        <div
          className="mt-3 rounded-[var(--radius-md)] border border-[var(--warning)] bg-[var(--warning-background)] p-4"
          role="alert"
        >
          <p className="text-sm font-semibold">
            {t("routines.workflow_project_mismatch_title")}
          </p>
          <p className="mt-1 text-xs leading-5">
            {t("routines.workflow_project_mismatch_help")}
          </p>
        </div>
      ) : null}

      <ScheduleBuilder
        disabled={disabled}
        initialScheduleText={initialScheduleText}
        key={initialScheduleText || "default"}
        onScheduleChange={onScheduleChange}
        t={t}
        timezone={timezone}
      />

      <div
        aria-live="polite"
        className="mt-5 rounded-[var(--radius-md)] bg-[var(--accent-background)] p-4"
      >
        {currentProposal ? (
          <>
            <p className="font-semibold">
              {humanScheduleSummary(
                currentProposal.scheduleExpression,
                currentProposal.timezone,
                t,
              )}
            </p>
            <p className="mt-2 text-xs font-semibold text-[var(--foreground-muted)]">
              {t("routines.upcoming")}
            </p>
            <ol className="mt-1 grid gap-1 text-sm text-[var(--foreground-muted)]">
              {currentProposal.nextRunsMs.slice(0, 3).map((time) => (
                <li key={time}>
                  {formatRoutineTimestamp(time, currentProposal.timezone)}
                </li>
              ))}
            </ol>
          </>
        ) : (
          <p className="text-sm text-[var(--foreground-muted)]">
            {proposalBusy
              ? t("routines.preparing")
              : scheduleError || t("routines.preview_waiting")}
          </p>
        )}
        <div className="mt-3 flex flex-wrap items-center gap-2 text-xs text-[var(--foreground-muted)]">
          <span>
            {t("routines.times_shown", {
              zone: humanTimezoneLabel(timezone),
            })}
          </span>
          <button
            className="font-semibold underline"
            disabled={disabled}
            onClick={openAdvanced}
            type="button"
          >
            {t("routines.change")}
          </button>
        </div>
      </div>

      <label className="mt-7 grid gap-2 text-sm font-semibold">
        {t("routines.delivery_platform")}
        <select
          className="rounded-[var(--radius-sm)] border bg-[var(--background)] px-3 py-2 font-normal"
          disabled={disabled}
          onChange={(event) =>
            onDeliveryChange(event.target.value as ChannelPlatform | "")
          }
          value={deliveryPlatform}
        >
          <option value="">{t("routines.local_only")}</option>
          {connectedChannels.map((status) => (
            <option key={status.platform} value={status.platform}>
              {t(`routines.delivery_${status.platform}`)}
            </option>
          ))}
        </select>
      </label>
      {destinationCopy ? (
        <label className="mt-3 grid gap-2 text-sm font-semibold">
          {t(destinationCopy.label)}
          <input
            className="rounded-[var(--radius-sm)] border bg-transparent px-3 py-2 font-normal"
            disabled={disabled}
            onChange={(event) =>
              onDeliveryDestinationChange(event.target.value)
            }
            placeholder={t(destinationCopy.placeholder)}
            value={deliveryDestination}
          />
        </label>
      ) : null}
      {connectedChannels.length < 3 ? (
        <button
          className="mt-2 text-xs font-semibold text-[var(--foreground-muted)] underline"
          disabled={disabled}
          onClick={onOpenConnections}
          type="button"
        >
          {t("routines.connect_messaging")}
        </button>
      ) : null}

      <label className="mt-7 grid gap-2 text-sm font-semibold">
        {t("routines.name_optional")}
        <input
          className="rounded-[var(--radius-sm)] border bg-transparent px-3 py-2 font-normal"
          disabled={disabled}
          onChange={(event) => onLabelChange(event.target.value)}
          value={label}
        />
      </label>

      <details
        className="mt-6 border-t border-[var(--border-soft)] pt-4"
        ref={advancedRef}
      >
        <summary className="w-fit cursor-pointer text-sm font-semibold">
          {t("routines.advanced")}
        </summary>
        <div className="mt-4 grid gap-4 rounded-[var(--radius-md)] bg-[var(--fill-hover)] p-4">
          <label className="grid gap-2 text-sm font-semibold">
            {t("routines.timezone")}
            <select
              className="rounded-[var(--radius-sm)] border bg-[var(--background)] px-3 py-2 font-normal"
              disabled={disabled}
              onChange={(event) => onTimezoneChange(event.target.value)}
              value={timezone}
            >
              {timezoneOptions.map((zone) => (
                <option key={zone} value={zone}>
                  {humanTimezoneLabel(zone)}
                </option>
              ))}
            </select>
          </label>
          <label className="grid gap-2 text-sm font-semibold">
            {t("routines.missed_policy")}
            <select
              className="rounded-[var(--radius-sm)] border bg-[var(--background)] px-3 py-2 font-normal"
              disabled={disabled}
              onChange={(event) => onMissedPolicyChange(event.target.value)}
              value={missedPolicy}
            >
              <option value="skip">{t("routines.missed_skip")}</option>
              <option value="run_once">{t("routines.missed_once")}</option>
              <option value="run_each">{t("routines.missed_each")}</option>
            </select>
          </label>
        </div>
      </details>

      {selectedWorkflow && selectedProject && !workflowPreparationRequired ? (
        <RoutineAuthorityReview
          deliveryDestination={deliveryDestination}
          deliveryPlatform={deliveryPlatform}
          disabled={disabled}
          onReviewWorkflow={() =>
            onOpenProjectWorkflows("saved_workflows")
          }
          projectName={selectedProject.name}
          reviewCapabilities={reviewCapabilities}
          t={t}
          workflowName={selectedWorkflow.name}
        />
      ) : null}

      <RoutineConfirmAction
        currentProposal={currentProposal}
        deliveryDestination={deliveryDestination}
        deliveryHint={destinationCopy?.hint ?? ""}
        deliveryPlatform={deliveryPlatform}
        disabled={disabled}
        isCreating={isCreating}
        onCreate={onCreate}
        projectId={projectId}
        proposalBusy={proposalBusy}
        reviewReady={reviewCapabilities.status === "ready"}
        t={t}
        workflowId={workflowId}
        workflowMatchesProject={workflowMatchesProject}
        workflowPreparationRequired={workflowPreparationRequired}
      />
    </div>
  );
}
