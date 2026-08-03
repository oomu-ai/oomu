"use client";

import { useI18n } from "@/context/I18nContext";
import { useId, useState } from "react";
import type { RoutineHistoryItem, RoutineRecord } from "./routineClient";

type CopyState = "idle" | "copying" | "copied" | "failed";

export type RoutineVerificationHistoryItem = RoutineHistoryItem & {
  taskRunId: string;
  taskId: string;
  runtimeRecordId: string;
  executionInstanceId: string;
  correlationId: string;
  scheduledForMs: number | null;
  runCreatedAtMs: number;
  scheduleCreatedAtMs: number;
  scheduleUpdatedAtMs: number;
  scheduleNextRunAtMs: number | null;
  effects: NonNullable<RoutineHistoryItem["effects"]>;
  deliveryReceipts: NonNullable<RoutineHistoryItem["deliveryReceipts"]>;
};

type RoutineVerificationRecordProps = {
  disabled?: boolean;
  item: RoutineVerificationHistoryItem;
  routine: RoutineRecord;
};

export function hasRoutineVerificationRecord(
  item: RoutineHistoryItem,
): item is RoutineVerificationHistoryItem {
  return Boolean(
    item.taskRunId &&
      item.taskId &&
      item.runtimeRecordId &&
      item.executionInstanceId &&
      item.correlationId &&
      typeof item.runCreatedAtMs === "number" &&
      typeof item.scheduleCreatedAtMs === "number" &&
      typeof item.scheduleUpdatedAtMs === "number" &&
      item.scheduledForMs !== undefined &&
      item.scheduleNextRunAtMs !== undefined &&
      Array.isArray(item.effects) &&
      Array.isArray(item.deliveryReceipts),
  );
}

export function buildRoutineVerificationRecord(
  routine: RoutineRecord,
  item: RoutineVerificationHistoryItem,
) {
  // Keep this whitelist explicit. Routine destinations, result prose, event
  // payloads, and provider receipts must never enter the copied record.
  return {
    schemaVersion: 1,
    routine: {
      routineId: routine.routineId,
      workflowId: routine.workflowId,
      workflowVersion: routine.workflowVersion ?? null,
      scheduleKind: routine.scheduleKind,
      state: routine.isActive ? "active" : "paused",
      createdAtMs: item.scheduleCreatedAtMs,
      updatedAtMs: item.scheduleUpdatedAtMs,
      nextRunAtMs: item.scheduleNextRunAtMs,
    },
    run: {
      executionInstanceId: item.executionInstanceId,
      taskRunId: item.taskRunId,
      taskId: item.taskId,
      runtimeRecordId: item.runtimeRecordId,
      correlationId: item.correlationId,
      state: item.state,
      scheduledForMs: item.scheduledForMs,
      createdAtMs: item.runCreatedAtMs,
      taskCreatedAtMs: item.createdAtMs,
      taskUpdatedAtMs: item.updatedAtMs,
    },
    effects: item.effects.map((effect) => ({
      idempotencyKey: effect.idempotencyKey,
      effectKind: effect.effectKind,
      state: effect.state,
      resultDigest: effect.resultDigest ?? null,
      updatedAtMs: effect.updatedAtMs,
    })),
    deliveryReceipts: item.deliveryReceipts.map((receipt) => ({
      receiptId: receipt.receiptId,
      platform: receipt.platform,
      eventKind: receipt.eventKind,
      state: receipt.state,
      providerMessageIdHash: receipt.providerReceiptHash ?? null,
      errorCode: receipt.errorCode ?? null,
      createdAtMs: receipt.createdAtMs,
      updatedAtMs: receipt.updatedAtMs,
    })),
  };
}

export function RoutineVerificationRecord({
  disabled = false,
  item,
  routine,
}: RoutineVerificationRecordProps) {
  const { t } = useI18n();
  const [copyState, setCopyState] = useState<CopyState>("idle");
  const statusId = useId();

  async function copyRecord() {
    setCopyState("copying");
    try {
      const record = buildRoutineVerificationRecord(routine, item);
      await navigator.clipboard.writeText(JSON.stringify(record, null, 2));
      setCopyState("copied");
    } catch {
      setCopyState("failed");
    }
  }

  const feedback =
    copyState === "copied"
      ? t("routines.verification_record_copied")
      : copyState === "failed"
        ? t("routines.verification_record_copy_failed")
        : "";

  return (
    <span className="flex flex-col items-end gap-1">
      <button
        aria-busy={copyState === "copying"}
        aria-describedby={statusId}
        className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] px-2.5 py-1.5 text-xs font-semibold transition-colors hover:bg-[var(--fill-hover)] disabled:cursor-not-allowed disabled:opacity-50"
        disabled={disabled || copyState === "copying"}
        onClick={() => void copyRecord()}
        type="button"
      >
        {copyState === "copied"
          ? t("common.copied")
          : t("routines.copy_verification_record")}
      </button>
      <span
        className={
          copyState === "failed"
            ? "max-w-64 text-right text-xs text-[var(--destructive)]"
            : "max-w-64 text-right text-xs text-[var(--success)]"
        }
        id={statusId}
        role={copyState === "failed" ? "alert" : "status"}
      >
        {feedback}
      </span>
    </span>
  );
}
