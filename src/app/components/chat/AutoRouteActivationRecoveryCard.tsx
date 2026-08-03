import { useEffect, useRef, useState } from "react";

export type AutoRouteActivationFailure = {
  sessionId: string;
  code: string;
  retryable: boolean;
  desiredEnabled: boolean;
};

type Translate = (key: string, variables?: Record<string, string | number>) => string;

export function AutoRouteActivationRecoveryCard({
  failure,
  onChooseModel,
  onDismiss,
  onRetry,
  t,
}: {
  failure: AutoRouteActivationFailure;
  onChooseModel?: () => void;
  onDismiss: () => void;
  onRetry: () => void | Promise<void>;
  t: Translate;
}) {
  const cardRef = useRef<HTMLElement>(null);
  const [retrying, setRetrying] = useState(false);
  const needsChoice = failure.desiredEnabled && [
    "auto_route_baseline_incomplete",
    "auto_route_local_provider_required",
    "auto_route_model_identity_invalid",
    "auto_route_provider_choice_required",
    "auto_route_provider_configuration_missing",
    "auto_route_provider_identity_mismatch",
    "auto_route_provider_model_mismatch",
    "auto_route_provider_not_local",
  ].includes(failure.code);
  const enabling = failure.desiredEnabled;
  const showRetry = failure.retryable || !needsChoice;

  useEffect(() => {
    cardRef.current?.focus({ preventScroll: true });
  }, [failure.code, failure.desiredEnabled, failure.sessionId]);

  async function retry() {
    if (retrying) return;
    setRetrying(true);
    try {
      await onRetry();
    } finally {
      setRetrying(false);
    }
  }

  return (
    <section
      aria-labelledby="auto-route-activation-recovery-title"
      className="max-w-3xl self-start rounded-[var(--radius-lg)] border border-[var(--warning)] bg-[var(--warning-background)] px-5 py-4 text-[var(--foreground)] outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)]"
      data-auto-route-activation-recovery="true"
      data-testid="auto-route-activation-recovery"
      id="oomu-auto-route-recovery"
      ref={cardRef}
      role="alert"
      tabIndex={-1}
    >
      <h3 className="text-sm font-semibold" id="auto-route-activation-recovery-title">
        {t("auto_route_activation.title")}
      </h3>
      <p className="mt-1 text-sm leading-6 text-[var(--foreground-muted)]">
        {t(needsChoice && enabling
          ? "auto_route_activation.choose_model_body"
          : enabling
            ? "auto_route_activation.enable_body"
            : "auto_route_activation.disable_body")}
      </p>
      <div className="mt-4 flex flex-wrap gap-2">
        {showRetry ? (
          <button
            aria-busy={retrying}
            className="inline-flex min-h-10 items-center justify-center rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-4 py-2 text-sm font-medium text-[var(--inverse-foreground)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)] disabled:opacity-40"
            data-auto-route-activation-action="retry"
            disabled={retrying}
            id="auto-route-activation-retry"
            onClick={() => void retry()}
            type="button"
          >
            {t(retrying
              ? "auto_route_activation.working"
              : "auto_route_activation.retry")}
          </button>
        ) : null}
        {needsChoice && onChooseModel ? (
          <button
            className="inline-flex min-h-10 items-center justify-center rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-4 py-2 text-sm font-medium text-[var(--inverse-foreground)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)]"
            data-auto-route-activation-action="choose_model"
            id="auto-route-activation-choose-model"
            onClick={onChooseModel}
            type="button"
          >
            {t("auto_route_activation.choose_model")}
          </button>
        ) : null}
        <button
          className="inline-flex min-h-10 items-center justify-center rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-4 py-2 text-sm font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)]"
          data-auto-route-activation-action="keep_route"
          id="auto-route-activation-keep-route"
          onClick={onDismiss}
          type="button"
        >
          {t(enabling
            ? "auto_route_activation.keep_current_model"
            : "auto_route_activation.leave_auto_route_on")}
        </button>
      </div>
      <details className="mt-3 text-xs text-[var(--foreground-muted)]">
        <summary className="cursor-pointer font-semibold focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)]">
          {t("auto_route_activation.technical_details")}
        </summary>
        <dl className="mt-2 grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--background)] p-3">
          <dt>{t("auto_route_activation.error_code")}</dt>
          <dd className="min-w-0 break-all font-mono text-[var(--foreground)]">{failure.code}</dd>
        </dl>
      </details>
    </section>
  );
}
