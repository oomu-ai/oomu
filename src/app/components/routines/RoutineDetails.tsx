"use client";

import type { RefObject } from "react";
import { RoutineDeliveryStatus } from "./RoutineDeliveryStatus";
import { routineDeliveryBlocksControls } from "./routineDeliveryControl";
import { RoutineHistoryOutcome } from "./RoutineHistoryOutcome";
import { RoutineIdentityDetails } from "./RoutineIdentityDetails";
import {
  hasRoutineVerificationRecord,
  RoutineVerificationRecord,
} from "./RoutineVerificationRecord";
import {
  formatRoutineTimestamp,
  humanScheduleSummary,
  humanTimezoneLabel,
  routineHistoryTime,
  routinePausedReasonLabel,
  shouldShowRoutinePausedReason,
  type RoutineTranslate,
} from "./routineLabels";
import type { RoutineHistoryItem, RoutineRecord } from "./routineClient";

type RoutineDetailsProps = {
  busyAction: string;
  deleteTriggerRef: RefObject<HTMLButtonElement | null>;
  history: RoutineHistoryItem[];
  historyBusy: boolean;
  interactionLocked: boolean;
  onDelete: () => void;
  onDuplicate: () => void;
  onOpenTask: (taskRunId: string, state: string) => void;
  onRefreshHistory: () => void;
  onRetryDelivery: () => void;
  onRunNow: () => void;
  onToggleActive: () => void;
  routine: RoutineRecord;
  t: RoutineTranslate;
};

export function RoutineDetails({
  busyAction,
  deleteTriggerRef,
  history,
  historyBusy,
  interactionLocked,
  onDelete,
  onDuplicate,
  onOpenTask,
  onRefreshHistory,
  onRetryDelivery,
  onRunNow,
  onToggleActive,
  routine,
  t,
}: RoutineDetailsProps) {
  return (
    <div className="mx-auto max-w-3xl">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h2 className="text-2xl font-semibold">{routine.label}</h2>
          <p className="mt-2 text-sm text-[var(--foreground-muted)]">
            {humanScheduleSummary(
              routine.scheduleExpression,
              routine.timezone,
              t,
            )}{" "}
            <span aria-hidden="true">·</span>{" "}
            {humanTimezoneLabel(routine.timezone)}
          </p>
        </div>
        <span className="rounded-full bg-[var(--accent-background)] px-3 py-1 text-xs">
          {routine.deliveryState === "retrying"
            ? t("routines.delivery_retrying_list")
            : routine.deliveryState === "needs_review"
              ? t("routines.delivery_review_list")
              : routine.isActive
                ? t("routines.active")
                : t("routines.paused")}
        </span>
      </div>
      {routine.pausedReason &&
      shouldShowRoutinePausedReason(routine.deliveryState) ? (
        <p className="mt-5 rounded bg-[var(--warning-background)] p-4 text-sm">
          {routinePausedReasonLabel(t, routine.pausedReason)}
        </p>
      ) : null}
      <RoutineIdentityDetails routine={routine} />
      <RoutineDeliveryStatus
        busy={busyAction === "delivery"}
        disabled={interactionLocked}
        onRetry={onRetryDelivery}
        state={routine.deliveryState}
        t={t}
      />
      <div className="mt-6 flex flex-wrap gap-2">
        <button
          className="rounded border px-3 py-2 text-sm font-semibold disabled:opacity-50"
          disabled={
            interactionLocked ||
            routineDeliveryBlocksControls(routine.deliveryState)
          }
          onClick={onRunNow}
          type="button"
        >
          {busyAction === "run"
            ? t("routines.running")
            : t("routines.run_now")}
        </button>
        <button
          className="rounded border px-3 py-2 text-sm disabled:opacity-50"
          disabled={
            interactionLocked ||
            routineDeliveryBlocksControls(routine.deliveryState)
          }
          onClick={onToggleActive}
          type="button"
        >
          {busyAction === "pause"
            ? t("routines.pausing")
            : busyAction === "resume"
              ? t("routines.resuming")
              : routine.isActive
                ? t("routines.pause")
                : t("routines.resume")}
        </button>
        <button
          className="rounded border px-3 py-2 text-sm disabled:opacity-50"
          disabled={interactionLocked}
          onClick={onDuplicate}
          type="button"
        >
          {busyAction === "duplicate"
            ? t("routines.duplicating")
            : t("routines.duplicate")}
        </button>
      </div>
      <div className="mt-7">
        <h3 className="text-sm font-semibold">{t("routines.upcoming")}</h3>
        <ol className="mt-3 grid gap-2">
          {routine.nextRunsMs.map((time) => (
            <li className="rounded border p-3 text-sm" key={time}>
              {formatRoutineTimestamp(time, routine.timezone)}
            </li>
          ))}
        </ol>
      </div>
      <div className="mt-7">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-semibold">{t("routines.history")}</h3>
          <button
            className="text-xs font-semibold disabled:opacity-50"
            disabled={interactionLocked}
            onClick={onRefreshHistory}
            type="button"
          >
            {historyBusy ? t("common.refreshing") : t("common.refresh")}
          </button>
        </div>
        <div className="mt-3 grid gap-2">
          {history.length === 0 ? (
            <p className="text-sm text-[var(--foreground-muted)]">
              {t("routines.no_history")}
            </p>
          ) : (
            history.map((item, index) => {
              const ranAt = routineHistoryTime(t, item.createdAtMs);
              return (
                <div
                  className="flex flex-wrap items-center gap-2 rounded border border-[var(--border-soft)] p-3 text-sm"
                  key={item.taskRunId || `${item.createdAtMs}-${index}`}
                >
                  <time dateTime={ranAt.dateTime}>{ranAt.label}</time>
                  <span aria-hidden="true">·</span>
                  <RoutineHistoryOutcome item={item} t={t} />
                  {item.taskRunId ? (
                    <div className="ml-auto flex flex-wrap items-start justify-end gap-2">
                      <button
                        className="px-2.5 py-1.5 font-semibold underline disabled:opacity-50"
                        disabled={interactionLocked}
                        onClick={() => onOpenTask(item.taskRunId!, item.state)}
                        type="button"
                      >
                        {t("routines.open_result")}
                      </button>
                      {hasRoutineVerificationRecord(item) ? (
                        <RoutineVerificationRecord
                          disabled={interactionLocked}
                          item={item}
                          routine={routine}
                        />
                      ) : null}
                    </div>
                  ) : null}
                </div>
              );
            })
          )}
        </div>
      </div>
      <div className="mt-8 border-t border-[var(--border-soft)] pt-6">
        <button
          className="rounded border border-[var(--destructive)] px-4 py-2 text-sm font-semibold text-[var(--destructive)] transition-colors hover:bg-[var(--destructive-background)] disabled:opacity-50"
          disabled={interactionLocked}
          onClick={onDelete}
          ref={deleteTriggerRef}
          type="button"
        >
          {t("routines.delete")}
        </button>
      </div>
    </div>
  );
}
