import type { RoutineDraft } from "./routineDraft";
import type { RoutineTranslate } from "./routineLabels";

export function RoutineHandoffNotice({
  draft,
  t,
}: {
  draft: RoutineDraft;
  t: RoutineTranslate;
}) {
  return (
    <section
      aria-label={t("routines.handoff_request_title")}
      className="mb-5 rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--fill-hover)] p-4"
    >
      <h2 className="text-sm font-semibold">
        {t("routines.handoff_request_title")}
      </h2>
      <p className="mt-2 text-sm">{draft.requestText}</p>
      <p className="mt-2 text-xs text-[var(--foreground-muted)]">
        {t(
          draft.scheduleSupported
            ? "routines.handoff_schedule_seeded"
            : "routines.handoff_schedule_needs_clarification",
        )}
      </p>
      {draft.timingDefaulted ? (
        <p className="mt-2 text-xs font-medium text-[var(--warning)]">
          {t("routines.handoff_timing_defaulted")}
        </p>
      ) : null}
      {draft.cadenceBoundaryConflict ? (
        <p className="mt-2 text-xs font-medium text-[var(--warning)]">
          {t("routines.handoff_cadence_boundary_conflict")}
        </p>
      ) : null}
      {!draft.scheduleSupported ? (
        <p className="mt-2 text-xs font-medium text-[var(--warning)]">
          {t("routines.handoff_schedule_unsupported")}
        </p>
      ) : null}
      {draft.runOnceRequested ? (
        <p className="mt-2 text-xs font-medium text-[var(--warning)]">
          {t("routines.handoff_run_once_pending")}
        </p>
      ) : null}
      {draft.endBoundary === "midnight" ? (
        <p className="mt-2 text-xs font-medium text-[var(--warning)]">
          {t("routines.handoff_midnight_enforced")}
        </p>
      ) : null}
    </section>
  );
}
