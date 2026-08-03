"use client";

import { useI18n } from "@/context/I18nContext";
import type { WorkflowIr } from "./workflowIr";
import type {
  WorkflowCompletion,
  WorkflowNodePayload,
} from "./workflowPersistence";

// The run report is the storyboard with results filled in: one row per executed
// step, in plain language, with the raw output tucked behind a disclosure. It
// replaces the old "Inspect → Run trace" JSON dump for everyone.
export function RunReport({
  completion,
  durationMs,
  executionOrder,
  nodePayloads,
  nodes,
  status,
}: {
  completion?: WorkflowCompletion;
  durationMs: number | null;
  executionOrder: string[];
  nodePayloads: Record<string, WorkflowNodePayload>;
  nodes: WorkflowIr["nodes"];
  status: string;
}) {
  const { t } = useI18n();
  const completedEmpty =
    status === "Completed" && completion?.kind === "empty_collection";
  const nodeById = new Map(nodes.map((node) => [node.id, node]));
  const orderedNodeIds = [
    ...executionOrder,
    ...nodes
      .map((node) => node.id)
      .filter(
        (nodeId) =>
          !executionOrder.includes(nodeId) && nodePayloads[nodeId] !== undefined,
      ),
  ];
  const steps = orderedNodeIds.flatMap((nodeId) => {
    const node = nodeById.get(nodeId);
    if (!node || node.kind === "input" || node.kind === "output") {
      return [];
    }
    return [{ node, payload: nodePayloads[nodeId] }];
  });

  return (
    <section className="mt-4 rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-4">
      <div className="flex items-center justify-between gap-3">
        <p className="text-sm font-semibold text-[var(--foreground)]">
          {t("workflows.run.title")}
        </p>
        <div className="flex shrink-0 items-center gap-2 text-xs font-medium text-[var(--foreground-muted)]">
          <span
            className={
              completedEmpty
                ? "rounded-full bg-[var(--success-background)] px-2 py-1 text-[var(--success)]"
                : undefined
            }
          >
            {runStatusLabel(status, t, completedEmpty)}
          </span>
          {durationMs !== null ? <span>{formatDuration(durationMs)}</span> : null}
        </div>
      </div>

      {completedEmpty ? (
        <p className="mt-2 text-xs leading-5 text-[var(--foreground-muted)]">
          {t("workflows.run.empty_result")}
        </p>
      ) : null}

      {steps.length === 0 && !completedEmpty ? (
        <p className="mt-2 text-xs leading-5 text-[var(--foreground-muted)]">
          {t("workflows.run.no_steps")}
        </p>
      ) : steps.length > 0 ? (
        <ol className="mt-3 grid gap-2">
          {steps.map(({ node, payload }, index) => {
            const outputText = formatWorkflowOutput(payload?.output);
            const errorText =
              payload?.status === "Failed"
                ? formatWorkflowError(payload?.error)
                : "";
            return (
              <li
                className="rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--background)] px-3 py-2"
                key={node.id}
              >
                <div className="flex items-start justify-between gap-3">
                  <p className="min-w-0 text-xs font-semibold text-[var(--foreground)]">
                    <span className="text-[var(--foreground-subtle)]">{index + 1}. </span>
                    {node.label}
                  </p>
                  <span
                    className={`shrink-0 text-[11px] font-semibold ${stepStatusClass(payload?.status)}`}
                  >
                    {stepStatusLabel(payload?.status, t)}
                  </span>
                </div>
                {errorText ? (
                  <p className="mt-1.5 text-xs leading-5 text-[var(--destructive)]">
                    {errorText}
                  </p>
                ) : null}
                {outputText ? (
                  <details className="group mt-1.5">
                    <summary className="cursor-pointer list-none text-[11px] font-medium text-[var(--foreground-muted)] transition-colors hover:text-[var(--foreground)] [&::-webkit-details-marker]:hidden">
                      {t("workflows.run.view_output")}
                    </summary>
                    <p className="mt-1.5 max-h-40 overflow-y-auto whitespace-pre-wrap break-words rounded-[var(--radius-sm)] bg-[var(--accent-background)] p-2 text-[11px] leading-5 text-[var(--foreground-muted)]">
                      {outputText}
                    </p>
                  </details>
                ) : null}
              </li>
            );
          })}
        </ol>
      ) : null}
    </section>
  );
}

function runStatusLabel(
  status: string,
  t: (key: string) => string,
  completedEmpty: boolean,
) {
  switch (status) {
    case "Completed":
      return t(
        completedEmpty
          ? "workflows.run.status.completed_empty"
          : "workflows.run.status.completed",
      );
    case "Failed":
      return t("workflows.run.status.failed");
    case "AwaitingApproval":
      return t("workflows.run.status.awaiting");
    case "Running":
      return t("workflows.run.status.running");
    default:
      return t("workflows.run.status.pending");
  }
}

function stepStatusLabel(status: string | undefined, t: (key: string) => string) {
  switch (status) {
    case "Completed":
      return t("workflows.run.step.done");
    case "Failed":
      return t("workflows.run.step.failed");
    case "Running":
      return t("workflows.run.step.running");
    default:
      return t("workflows.run.step.skipped");
  }
}

function stepStatusClass(status: string | undefined) {
  switch (status) {
    case "Completed":
      return "text-[var(--success)]";
    case "Failed":
      return "text-[var(--destructive)]";
    default:
      return "text-[var(--foreground-subtle)]";
  }
}

function formatWorkflowError(error: unknown): string {
  if (
    error &&
    typeof error === "object" &&
    "message" in error &&
    typeof (error as { message?: unknown }).message === "string"
  ) {
    return (error as { message: string }).message.trim();
  }
  return formatWorkflowOutput(error);
}

export function formatWorkflowOutput(output: unknown): string {
  if (output === null || output === undefined) {
    return "";
  }
  if (typeof output === "string") {
    return output.trim();
  }
  if (typeof output === "number" || typeof output === "boolean") {
    return String(output);
  }
  if (Array.isArray(output)) {
    return output
      .slice(0, 20)
      .map((value, index) => `${index + 1}. ${formatWorkflowOutput(value)}`)
      .filter((value) => !value.endsWith(". "))
      .join("\n");
  }
  if (typeof output === "object") {
    return Object.entries(output as Record<string, unknown>)
      .slice(0, 30)
      .flatMap(([key, value]) => {
        const formatted = formatWorkflowOutput(value);
        return formatted ? [`${humanizeOutputKey(key)}: ${formatted}`] : [];
      })
      .join("\n");
  }
  return "";
}

function humanizeOutputKey(key: string) {
  return key
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/[_-]+/g, " ")
    .replace(/^./, (character) => character.toUpperCase());
}

function formatDuration(durationMs: number) {
  if (durationMs < 1000) {
    return `${Math.max(1, Math.round(durationMs))} ms`;
  }
  return `${(durationMs / 1000).toFixed(1)} s`;
}
