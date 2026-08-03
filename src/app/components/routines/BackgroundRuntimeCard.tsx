import type { BackgroundStatus } from "./routineClient";
import type { RoutineTranslate } from "./routineLabels";

type Props = {
  busy: boolean;
  disabled: boolean;
  error: string;
  onChange: (enabled: boolean) => void;
  onOpenLoginItems: () => void;
  onRefresh: () => void;
  status: BackgroundStatus | null;
  t: RoutineTranslate;
};

const STATE_COPY: Record<string, { label: string; help: string }> = {
  off: {
    label: "routines.background_state_off",
    help: "routines.background_off_help",
  },
  turning_on: {
    label: "routines.background_state_turning_on",
    help: "routines.background_turning_on_help",
  },
  on_verified: {
    label: "routines.background_state_on",
    help: "routines.background_on_help",
  },
  needs_attention: {
    label: "routines.background_state_needs_attention",
    help: "routines.background_needs_attention_help",
  },
  turning_off: {
    label: "routines.background_state_turning_off",
    help: "routines.background_turning_off_help",
  },
};

function copyFor(status: BackgroundStatus | null) {
  if (!status) return null;
  return STATE_COPY[status.state] ?? STATE_COPY.needs_attention;
}

type RuntimeView = {
  actionDisabled: boolean;
  approvalRequired: boolean;
  helpKey?: string;
  needsAttention: boolean;
  signedInstallRequired: boolean;
  statusUnavailable: boolean;
  toggleDisabled: boolean;
  transitioning: boolean;
};

function runtimeView(
  busy: boolean,
  disabled: boolean,
  error: string,
  status: BackgroundStatus | null,
): RuntimeView {
  const statusUnavailable = !status && Boolean(error);
  const transitioning =
    status?.state === "turning_on" || status?.state === "turning_off";
  const approvalRequired = status?.errorCode === "background_requires_approval";
  const signedInstallRequired = status?.errorCode === "background_requires_signed_install";
  const actionDisabled = disabled || busy || transitioning;
  const copy = copyFor(status);
  const helpKey = statusUnavailable
    ? "routines.background_check_failed"
    : approvalRequired
      ? "routines.background_approval_help"
      : signedInstallRequired
        ? "routines.background_signed_install_help"
        : copy?.help;
  return {
    actionDisabled,
    approvalRequired,
    helpKey,
    needsAttention: status?.state === "needs_attention" || statusUnavailable,
    signedInstallRequired,
    statusUnavailable,
    toggleDisabled: actionDisabled || !status,
    transitioning,
  };
}

function RecoveryActions({
  busy,
  onChange,
  onOpenLoginItems,
  onRefresh,
  status,
  t,
  view,
}: Pick<Props, "busy" | "onChange" | "onOpenLoginItems" | "onRefresh" | "status" | "t"> & {
  view: RuntimeView;
}) {
  if (!view.needsAttention) return null;
  const repair = view.statusUnavailable
    ? onRefresh
    : view.approvalRequired
      ? onOpenLoginItems
      : () => onChange(true);
  const repairLabel = view.approvalRequired
    ? "routines.background_open_login_items"
    : "routines.background_try_again";
  return (
    <div
      aria-describedby="background-runtime-help"
      aria-label={t("routines.background_repair_label")}
      className="mt-3 flex flex-wrap gap-2"
      id="background-runtime-repair"
      role="group"
    >
      {!view.signedInstallRequired ? (
        <button
          className="rounded bg-[var(--inverse-background)] px-3 py-1.5 text-xs font-semibold text-[var(--inverse-foreground)] disabled:opacity-50"
          disabled={view.actionDisabled}
          id="oomu-background-repair"
          onClick={repair}
          type="button"
        >
          {busy ? t("routines.updating") : t(repairLabel)}
        </button>
      ) : null}
      {view.approvalRequired ? (
        <button
          className="rounded border px-3 py-1.5 text-xs disabled:opacity-50"
          disabled={view.actionDisabled}
          onClick={() => onChange(true)}
          type="button"
        >
          {t("routines.background_recheck")}
        </button>
      ) : null}
      {status || view.statusUnavailable ? (
        <button
          className="rounded border px-3 py-1.5 text-xs disabled:opacity-50"
          disabled={view.actionDisabled}
          onClick={() => onChange(false)}
          type="button"
        >
          {t("routines.background_turn_off")}
        </button>
      ) : null}
    </div>
  );
}

export function BackgroundRuntimeCard({
  busy,
  disabled,
  error,
  onChange,
  onOpenLoginItems,
  onRefresh,
  status,
  t,
}: Props) {
  const copy = copyFor(status);
  const view = runtimeView(busy, disabled, error, status);

  return (
    <section
      aria-labelledby="background-runtime-title"
      className="mt-5 rounded border p-3"
      id="background-runtime-control"
    >
      <div className="flex items-start justify-between gap-3">
        <div>
          <h2 className="text-sm font-semibold" id="background-runtime-title">
            {t("routines.background")}
          </h2>
          <p
            aria-live="polite"
            className="mt-1 text-xs font-medium"
            id="background-runtime-state"
          >
            {view.statusUnavailable
              ? t("routines.background_state_needs_attention")
              : copy
                ? t(copy.label)
                : t("common.loading")}
          </p>
        </div>
        {!view.needsAttention ? (
          <button
            aria-describedby="background-runtime-help"
            className="rounded border px-2 py-1 text-xs disabled:opacity-50"
            disabled={view.toggleDisabled}
            id="oomu-background-toggle"
            onClick={() => onChange(!status?.userEnabled)}
            type="button"
          >
            {busy || view.transitioning
              ? t("routines.updating")
              : status?.userEnabled
                ? t("routines.background_turn_off")
                : t("routines.background_turn_on")}
          </button>
        ) : null}
      </div>
      <p
        className="mt-1 text-xs text-[var(--foreground-muted)]"
        id="background-runtime-help"
        role={view.statusUnavailable ? "alert" : undefined}
      >
        {view.helpKey ? t(view.helpKey) : t("routines.background_checking_help")}
      </p>
      <RecoveryActions
        busy={busy}
        onChange={onChange}
        onOpenLoginItems={onOpenLoginItems}
        onRefresh={onRefresh}
        status={status}
        t={t}
        view={view}
      />
      {error && !view.statusUnavailable ? (
        <p className="mt-2 text-xs text-[var(--warning)]" role="alert">
          {error}
        </p>
      ) : null}
    </section>
  );
}
