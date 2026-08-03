import { routineHistoryState, type RoutineTranslate } from "./routineLabels";
import type { RoutineHistoryItem } from "./routineClient";

export function RoutineHistoryOutcome({
  item,
  t,
}: {
  item: RoutineHistoryItem;
  t: RoutineTranslate;
}) {
  if (item.state.toLowerCase() === "failed" && item.lastError) {
    const sourceFailure =
      item.lastErrorCode === "official_page_fetch_failed" ||
      /official (page|source)/i.test(item.lastError);
    return (
      <>
        <span>{routineHistoryState(t, item.state)}</span>
        <div className="w-full rounded-[var(--radius-sm)] bg-[var(--warning-background)] p-3">
          <p className="font-semibold">{t("routines.history_failure_title")}</p>
          <p className="mt-1 text-xs text-[var(--foreground-muted)]">
            {t(
              sourceFailure
                ? "routines.history_failure_official_source"
                : "routines.history_failure_generic",
            )}
          </p>
          <details className="mt-2 text-xs text-[var(--foreground-muted)]">
            <summary className="cursor-pointer font-semibold">
              {t("routines.history_failure_details")}
            </summary>
            <p className="mt-1 break-words">{item.lastError}</p>
          </details>
        </div>
      </>
    );
  }
  if (item.outcome !== "completed_with_declined_actions") {
    return <span>{routineHistoryState(t, item.state)}</span>;
  }
  return (
    <>
      <span>{t("routines.history_completed_with_declined_actions")}</span>
      {item.declinedActions?.length ? (
        <span className="w-full text-xs text-[var(--foreground-muted)]">
          {t("routines.history_declined_actions", {
            actions: item.declinedActions.join(", "),
          })}
        </span>
      ) : null}
    </>
  );
}
