import type { RoutineTranslate } from "./routineLabels";
import type { RoutineRecord } from "./routineClient";

type Props = {
  busy: boolean;
  disabled: boolean;
  onRetry: () => void;
  state: RoutineRecord["deliveryState"];
  t: RoutineTranslate;
};

export function RoutineDeliveryStatus({
  busy,
  disabled,
  onRetry,
  state,
  t,
}: Props) {
  if (state === "retrying") {
    return (
      <section
        aria-live="polite"
        className="mt-5 rounded-xl border border-[var(--border-soft)] bg-[var(--fill-subtle)] p-4"
      >
        <h3 className="text-sm font-semibold">
          {t("routines.delivery_retrying_title")}
        </h3>
        <p className="mt-1 text-sm text-[var(--foreground-muted)]">
          {t("routines.delivery_retrying_body")}
        </p>
      </section>
    );
  }
  if (state !== "needs_review") return null;

  return (
    <section
      className="mt-5 rounded-xl border border-[var(--warning)] bg-[var(--warning-background)] p-4"
      role="status"
    >
      <h3 className="text-sm font-semibold">
        {t("routines.delivery_review_title")}
      </h3>
      <p className="mt-1 text-sm">{t("routines.delivery_review_body")}</p>
      <button
        className="mt-3 rounded bg-[var(--inverse-background)] px-3 py-2 text-sm font-semibold text-[var(--inverse-foreground)] disabled:opacity-50"
        disabled={disabled}
        onClick={onRetry}
        type="button"
      >
        {busy
          ? t("routines.delivery_retrying_action")
          : t("routines.delivery_review_action")}
      </button>
    </section>
  );
}
