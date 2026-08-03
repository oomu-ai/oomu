"use client";

import type { WorkflowRunToast } from "./useWorkflowRun";

export function WorkflowRunFeedback({
  feedback,
}: {
  feedback: WorkflowRunToast | null;
}) {
  if (!feedback) {
    return null;
  }

  const urgent = feedback.tone === "error";
  const toneClasses =
    feedback.tone === "error"
      ? "border-[var(--destructive)] text-[var(--foreground)]"
      : feedback.tone === "success"
        ? "border-[var(--success)] text-[var(--foreground)]"
        : "border-[var(--border-strong)] text-[var(--foreground)]";
  const dotClasses =
    feedback.tone === "error"
      ? "bg-[var(--destructive)]"
      : feedback.tone === "success"
        ? "bg-[var(--success)]"
        : "bg-[var(--accent)]";

  return (
    <div className="shrink-0 border-b border-[var(--border-soft)] bg-[var(--background)] px-5 py-3">
      <div
        aria-live={urgent ? "assertive" : "polite"}
        className={`mx-auto flex max-w-4xl items-start gap-3 rounded-[var(--radius-sm)] border bg-[var(--background)] px-4 py-3 shadow-sm ${toneClasses}`}
        data-tone={feedback.tone}
        role={urgent ? "alert" : "status"}
      >
        <span
          aria-hidden="true"
          className={`mt-2 h-2 w-2 shrink-0 rounded-full ${dotClasses}`}
        />
        <p className="min-w-0 whitespace-pre-wrap break-words text-sm leading-6">
          {feedback.message}
        </p>
      </div>
    </div>
  );
}
