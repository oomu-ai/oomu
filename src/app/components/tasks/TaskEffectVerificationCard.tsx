import type {
  TaskEffectVerification,
  TaskEffectVerificationDecision,
} from "./taskEffectVerification";

type TranslateFn = (key: string, values?: Record<string, string | number>) => string;

export function TaskEffectVerificationCard({
  decision,
  detailsState,
  onDecision,
  onReload,
  t,
  verification,
}: {
  decision: TaskEffectVerificationDecision | "";
  detailsState: "loading" | "ready" | "error";
  onDecision: (decision: TaskEffectVerificationDecision) => void;
  onReload: () => void;
  t: TranslateFn;
  verification: TaskEffectVerification | null;
}) {
  if (!verification) {
    return (
      <section className="rounded-[var(--radius-md)] border border-[var(--warning)]/40 bg-[var(--warning-background)] p-5">
        <h3 className="text-base font-semibold">{t("tasks.effect_verification_title")}</h3>
        <p className="mt-2 text-sm text-[var(--foreground-muted)]">
          {t(
            detailsState === "loading"
              ? "tasks.effect_verification_loading"
              : "tasks.effect_verification_load_error",
          )}
        </p>
        {detailsState !== "loading" ? (
          <div className="mt-4 flex flex-wrap gap-2">
            <button
              className="rounded-[var(--radius-sm)] bg-[var(--accent)] px-4 py-2 text-sm font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)] disabled:cursor-wait disabled:opacity-50"
              disabled={Boolean(decision)}
              onClick={onReload}
              type="button"
            >
              {t("tasks.effect_verification_reload")}
            </button>
            <button
              aria-busy={decision === "stop_without_repeating"}
              className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-4 py-2 text-sm font-semibold transition-colors hover:bg-[var(--fill-hover)] disabled:cursor-wait disabled:opacity-50"
              disabled={Boolean(decision)}
              onClick={() => onDecision("stop_without_repeating")}
              type="button"
            >
              {decision === "stop_without_repeating"
                ? t("tasks.effect_verification_working")
                : t("tasks.effect_verification_stop_without_details")}
            </button>
          </div>
        ) : null}
      </section>
    );
  }

  const copy = verification.surface === "calendar"
    ? "calendar"
    : verification.surface === "mail_draft"
      ? "mail_draft"
      : verification.surface === "mail_send"
        ? "mail_send"
        : "generic";
  const details = verification.surface === "calendar"
    ? [
        [t("mcp_confirmation.calendar"), verification.calendarName],
        [t("mcp_confirmation.event_title"), verification.title],
      ]
    : [
        [t("mcp_confirmation.recipient"), verification.recipient],
        [t("mcp_confirmation.subject"), verification.subject],
      ];
  const busy = Boolean(decision);

  return (
    <section className="rounded-[var(--radius-md)] border border-[var(--warning)]/40 bg-[var(--warning-background)] p-5">
      <h3 className="text-base font-semibold">{t("tasks.effect_verification_title")}</h3>
      <p className="mt-2 text-sm">{t(`tasks.effect_verification_${copy}_body`)}</p>
      <p className="mt-2 text-sm font-medium">
        {t(`tasks.effect_verification_${copy}_inspect`)}
      </p>
      {details.some(([, value]) => value) ? (
        <dl className="mt-4 rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--background)] p-4 text-sm">
          {details.map(([label, value]) => value ? (
            <div className="grid grid-cols-[7rem_minmax(0,1fr)] gap-3 py-1" key={label}>
              <dt className="text-[var(--foreground-muted)]">{label}</dt>
              <dd className="break-words font-medium">{value}</dd>
            </div>
          ) : null)}
        </dl>
      ) : null}
      {!verification.retrySupported ? (
        <p className="mt-4 text-sm text-[var(--foreground-muted)]">
          {t("tasks.effect_verification_retry_unavailable")}
        </p>
      ) : null}
      <div className="mt-4 flex flex-wrap gap-2">
        {verification.retrySupported ? (
          <button
            aria-busy={decision === "did_not_happen"}
            className="rounded-[var(--radius-sm)] bg-[var(--accent)] px-4 py-2 text-sm font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)] disabled:cursor-wait disabled:opacity-50"
            disabled={busy}
            onClick={() => onDecision("did_not_happen")}
            type="button"
          >
            {decision === "did_not_happen"
              ? t("tasks.effect_verification_working")
              : t("tasks.effect_verification_did_not_happen")}
          </button>
        ) : null}
        <button
          aria-busy={decision === "happened"}
          className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-4 py-2 text-sm font-semibold transition-colors hover:bg-[var(--fill-hover)] disabled:cursor-wait disabled:opacity-50"
          disabled={busy}
          onClick={() => onDecision("happened")}
          type="button"
        >
          {decision === "happened"
            ? t("tasks.effect_verification_working")
            : t("tasks.effect_verification_happened")}
        </button>
        <button
          aria-busy={decision === "stop_without_repeating"}
          className="rounded-[var(--radius-sm)] border border-transparent px-4 py-2 text-sm font-semibold text-[var(--foreground-muted)] transition-colors hover:border-[var(--border-soft)] hover:bg-[var(--fill-hover)] hover:text-[var(--foreground)] disabled:cursor-wait disabled:opacity-50"
          disabled={busy}
          onClick={() => onDecision("stop_without_repeating")}
          type="button"
        >
          {decision === "stop_without_repeating"
            ? t("tasks.effect_verification_working")
            : t("tasks.effect_verification_stop_without_repeating")}
        </button>
      </div>
    </section>
  );
}
