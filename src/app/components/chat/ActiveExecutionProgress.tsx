"use client";

import { useI18n } from "@/context/I18nContext";
import { isDeveloperBuild } from "@/lib/buildFlags";
import { useHumanTrust } from "@/lib/utils/trustUtils";
import {
  executionStatusLabel,
  formatExecutionTime,
  type ActiveAgentExecution,
} from "./agentExecutionState";

export function ActiveExecutionProgress({
  execution,
  onTrackInTasks,
}: {
  execution: ActiveAgentExecution;
  onTrackInTasks?: () => void;
}) {
  const { t } = useI18n();
  const { getPhaseLabel } = useHumanTrust();
  const latestLog = execution.logs[execution.logs.length - 1] ?? null;
  const statusLabel = executionStatusLabel(execution.status, t);
  const isRunning = execution.status === "running";

  return (
    <section className="max-w-3xl self-start rounded-[var(--radius-md)] border border-[var(--border-strong)] bg-[var(--background)] text-[var(--foreground)]" data-agent-execution-id={isDeveloperBuild ? execution.executionId : undefined} data-agent-execution-status={execution.status}>
      <details className="group" open={isRunning}>
        <summary className="flex cursor-pointer list-none items-center justify-between gap-3 px-4 py-3 text-sm font-semibold [&::-webkit-details-marker]:hidden">
          <span className="min-w-0 truncate">
            {t("chat.execution.title", { status: statusLabel })}
          </span>
          <span className="shrink-0 text-xs font-medium text-[var(--foreground-muted)]">
            {execution.logs.length === 1
              ? t("chat.execution.step_one")
              : t("chat.execution.step_many", { count: execution.logs.length })}
          </span>
        </summary>
        <div className="border-t border-[var(--border-soft)] px-4 pb-4 pt-3">
          <div className="flex flex-wrap items-center gap-2 text-xs text-[var(--foreground-muted)]">
            {latestLog && <span>{formatExecutionTime(latestLog.createdAtMs)}</span>}
            {onTrackInTasks ? <button className="ml-auto font-semibold text-[var(--foreground)] underline decoration-[var(--border-strong)] underline-offset-4" onClick={onTrackInTasks} type="button">{t("common.track_in_tasks")}</button> : null}
          </div>
          {isDeveloperBuild ? (
            <details className="mt-3 text-xs text-[var(--foreground-muted)]">
              <summary className="cursor-pointer font-semibold text-[var(--foreground)]">
                {t("chat.execution.technical_details")}
              </summary>
              <p className="mt-2 break-all font-mono">
                {execution.executionId} · {execution.planId}
              </p>
              {execution.logs.length > 0 ? (
                <ol className="mt-2 grid gap-2 border-l border-[var(--border-soft)] pl-3">
                  {execution.logs.map((log) => (
                    <li key={log.id}>
                      <span className="font-semibold">{getPhaseLabel(log.phase)}</span>
                      <p className="mt-1 whitespace-pre-wrap break-words font-mono">
                        {log.message}
                      </p>
                    </li>
                  ))}
                </ol>
              ) : null}
            </details>
          ) : null}
          {isRunning && (
            <div className="mt-3 flex items-center gap-2 text-xs font-medium text-[var(--foreground-muted)]" role="status">
              <svg aria-hidden="true" className="h-4 w-4 animate-spin text-[var(--accent)]" fill="none" viewBox="0 0 24 24">
                <circle className="opacity-25" cx="12" cy="12" r="9" stroke="currentColor" strokeWidth="3" />
                <path className="opacity-90" d="M21 12a9 9 0 0 0-9-9" stroke="currentColor" strokeLinecap="round" strokeWidth="3" />
              </svg>
              <span>{t("chat.execution.working")}</span>
            </div>
          )}
          <ol className="mt-3 grid max-h-72 gap-2 overflow-y-auto pr-1">
            {execution.logs.length === 0 ? (
              <li className="rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--accent-background)] px-3 py-2 text-sm text-[var(--foreground-muted)]">
                {t("chat.execution.waiting_logs")}
              </li>
            ) : (
              execution.logs.map((log) => (
                <li
                  className={`rounded-[var(--radius-sm)] border px-3 py-2 text-sm leading-6 ${
                    log.level === "error"
                      ? "border-[var(--destructive)] bg-[var(--destructive-background)] text-[var(--destructive)]"
                      : "border-[var(--border-soft)] bg-[var(--accent-background)]"
                  }`}
                  key={log.id}
                >
                  <div className="flex flex-wrap items-center justify-between gap-2">
                    <span className="text-xs font-semibold text-[var(--foreground-subtle)]">
                      {getPhaseLabel(log.phase)}
                    </span>
                    <span className="text-[11px] text-[var(--foreground-muted)]">
                      {formatExecutionTime(log.createdAtMs)}
                    </span>
                  </div>
                </li>
              ))
            )}
          </ol>
        </div>
      </details>
    </section>
  );
}
