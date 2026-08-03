import { useEffect, useRef, useState } from "react";
import type { AutoRouteRecoveryKind } from "./autoRouteRecovery";
import {
  compactExecutionModelLabel,
  modelIdentityIsOpaque,
} from "./RoutingIndicator";

export type AutoRouteTurnChoice = "retry" | "local" | "cloud" | "cancel";
export type AutoRouteRecoveryAction =
  | AutoRouteTurnChoice
  | "repair_model"
  | "open_models"
  | "continue_when_ready"
  | "check_saved_work";

export type AutoRouteAttention = {
  sessionId: string;
  rootTurnId: string;
  turnId: string;
  generationToken: string;
  localProviderId: string;
  localModelId: string;
  recommendedLocalProviderId: string;
  recommendedLocalModelId: string;
  cloudModelId: string;
  failureCode: string;
  failureBoundary: string | null;
  kind: AutoRouteRecoveryKind;
  continueWhenReady?: boolean;
};

type Translate = (key: string, variables?: Record<string, string | number>) => string;

type AutoRouteAttentionCardProps = {
  attention: AutoRouteAttention;
  onChoice: (choice: AutoRouteRecoveryAction) => void | Promise<void>;
  t: Translate;
};

const kindCopyKeys: Record<AutoRouteRecoveryKind, { title: string; body: string }> = {
  choose_model: {
    title: "sprint_301.auto_route_recovery.choose_model_title",
    body: "sprint_301.auto_route_recovery.choose_model_body",
  },
  preparing: {
    title: "sprint_301.auto_route_recovery.preparing_title",
    body: "sprint_301.auto_route_recovery.preparing_body",
  },
  timeout: {
    title: "sprint_301.auto_route_recovery.timeout_title",
    body: "sprint_301.auto_route_recovery.timeout_body",
  },
  cloud_setup: {
    title: "sprint_301.auto_route_recovery.cloud_title",
    body: "sprint_301.auto_route_recovery.cloud_body",
  },
  saved_work_check: {
    title: "sprint_301.auto_route_recovery.saved_work_title",
    body: "sprint_301.auto_route_recovery.saved_work_body",
  },
  interrupted: {
    title: "sprint_301.auto_route_recovery.interrupted_title",
    body: "sprint_301.auto_route_recovery.interrupted_body",
  },
  unknown: {
    title: "sprint_301.auto_route_recovery.unknown_title",
    body: "sprint_301.auto_route_recovery.unknown_body",
  },
};

function secondaryButtonClass() {
  return "inline-flex min-h-10 items-center justify-center rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-4 py-2 text-sm font-medium transition-colors hover:bg-[var(--fill-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)] disabled:cursor-not-allowed disabled:opacity-40";
}

function primaryButtonClass() {
  return "inline-flex min-h-10 items-center justify-center rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-4 py-2 text-sm font-medium text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)] disabled:cursor-not-allowed disabled:opacity-40";
}

type RecoveryActionSpec = {
  action: AutoRouteRecoveryAction;
  label: string;
  primary: boolean;
};

function recoveryActionSpecs(
  attention: AutoRouteAttention,
  localLabel: string,
  hasVerifiedLocalModel: boolean,
  hasCloudModel: boolean,
  t: Translate,
) {
  const actions: RecoveryActionSpec[] = [];
  switch (attention.kind) {
    case "choose_model":
      actions.push({
        action: hasVerifiedLocalModel ? "repair_model" : "open_models",
        label: hasVerifiedLocalModel
          ? t("sprint_301.auto_route_recovery.use_local", { model: localLabel })
          : t("sprint_301.auto_route_recovery.choose_model"),
        primary: true,
      });
      break;
    case "preparing":
      if (!attention.continueWhenReady) {
        actions.push({
          action: "continue_when_ready",
          label: t("sprint_301.auto_route_recovery.continue_when_ready"),
          primary: true,
        });
      }
      break;
    case "timeout":
      actions.push({
        action: "retry",
        label: t("sprint_301.auto_route_recovery.try_again"),
        primary: true,
      });
      break;
    case "cloud_setup":
      actions.push({
        action: "open_models",
        label: t("sprint_301.auto_route_recovery.open_models"),
        primary: true,
      });
      break;
    case "saved_work_check":
      actions.push({
        action: "check_saved_work",
        label: t("sprint_301.auto_route_recovery.check_saved_work"),
        primary: true,
      });
      break;
    case "interrupted":
      actions.push({
        action: "retry",
        label: t("sprint_301.auto_route_recovery.continue_when_ready"),
        primary: true,
      });
      break;
    case "unknown":
      actions.push({
        action: "retry",
        label: t("sprint_301.auto_route_recovery.try_again"),
        primary: true,
      });
      break;
  }
  if (["timeout", "cloud_setup", "unknown"].includes(attention.kind) && hasVerifiedLocalModel) {
    actions.push({
      action: "local",
      label: t("sprint_301.auto_route_recovery.use_local", { model: localLabel }),
      primary: false,
    });
  }
  if (attention.kind === "unknown" && hasCloudModel) {
    actions.push({
      action: "cloud",
      label: t("sprint_301.auto_route_recovery.use_cloud"),
      primary: false,
    });
  }
  actions.push({
    action: "cancel",
    label: t("sprint_301.auto_route_recovery.cancel"),
    primary: false,
  });
  return actions;
}

function AutoRouteRecoveryControls({
  attention,
  hasCloudModel,
  hasVerifiedLocalModel,
  localLabel,
  onChoice,
  t,
}: AutoRouteAttentionCardProps & {
  hasCloudModel: boolean;
  hasVerifiedLocalModel: boolean;
  localLabel: string;
}) {
  const [pendingAction, setPendingAction] = useState<AutoRouteRecoveryAction | null>(null);
  const [actionFailed, setActionFailed] = useState(false);
  const actions = recoveryActionSpecs(
    attention,
    localLabel,
    hasVerifiedLocalModel,
    hasCloudModel,
    t,
  );

  async function perform(action: AutoRouteRecoveryAction) {
    if (pendingAction) return;
    setActionFailed(false);
    setPendingAction(action);
    try {
      await onChoice(action);
    } catch {
      setActionFailed(true);
    } finally {
      setPendingAction(null);
    }
  }

  return (
    <>
      <div className="mt-4 flex flex-wrap gap-2">
        {actions.map((item) => (
          <button
            className={item.primary ? primaryButtonClass() : secondaryButtonClass()}
            data-auto-route-choice={item.action}
            disabled={Boolean(pendingAction)}
            key={item.action}
            onClick={() => void perform(item.action)}
            type="button"
          >
            {pendingAction !== null && pendingAction === item.action
              ? t("sprint_301.auto_route_recovery.working")
              : item.label}
          </button>
        ))}
      </div>
      {attention.kind === "preparing" && attention.continueWhenReady ? (
        <p className="mt-3 text-xs leading-5 text-[var(--foreground-muted)]" id="auto-route-preparing-help">
          {t("sprint_301.auto_route_recovery.preparing_wait")}
        </p>
      ) : null}
      {attention.kind === "unknown" && hasCloudModel ? (
        <p className="mt-3 text-xs leading-5 text-[var(--foreground-muted)]">
          {t("sprint_301.auto_route_recovery.cloud_disclosure")}
        </p>
      ) : null}
      {actionFailed ? (
        <p className="mt-3 text-xs font-medium text-[var(--destructive)]" role="alert">
          {t("sprint_301.auto_route_recovery.action_failed")}
        </p>
      ) : null}
    </>
  );
}

function AutoRouteTechnicalDetails({ attention, t }: Pick<AutoRouteAttentionCardProps, "attention" | "t">) {
  return (
    <details className="mt-3 text-xs text-[var(--foreground-muted)]">
      <summary className="cursor-pointer font-semibold outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)]">
        {t("sprint_301.auto_route_recovery.technical_details")}
      </summary>
      <dl className="mt-2 grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--background)] p-3">
        <dt>{t("sprint_301.auto_route_recovery.error_code")}</dt>
        <dd className="min-w-0 break-all font-mono text-[var(--foreground)]">{attention.failureCode}</dd>
        {attention.failureBoundary ? (
          <>
            <dt>{t("sprint_301.auto_route_recovery.stopped_at")}</dt>
            <dd className="min-w-0 break-all font-mono text-[var(--foreground)]">{attention.failureBoundary}</dd>
          </>
        ) : null}
      </dl>
    </details>
  );
}

export function AutoRouteAttentionCard({ attention, onChoice, t }: AutoRouteAttentionCardProps) {
  const cardRef = useRef<HTMLElement>(null);
  const copy = kindCopyKeys[attention.kind];
  const localModelId = attention.recommendedLocalModelId || attention.localModelId;
  const hasVerifiedLocalModel = Boolean(localModelId) && !modelIdentityIsOpaque(localModelId);
  const localLabel = compactExecutionModelLabel(
    localModelId,
    t("sprint_301.route.on_device_model"),
  );
  const hasCloudModel = Boolean(attention.cloudModelId.trim());
  const isPreparing = attention.kind === "preparing";

  useEffect(() => {
    cardRef.current?.focus({ preventScroll: true });
  }, [
    attention.failureCode,
    attention.generationToken,
    attention.sessionId,
    attention.turnId,
  ]);

  return (
    <section
      aria-labelledby="auto-route-attention-title"
      aria-live={isPreparing ? "polite" : "assertive"}
      className="max-w-3xl self-start rounded-[var(--radius-lg)] border border-[var(--warning)] bg-[var(--warning-background)] px-5 py-4 text-[var(--foreground)] outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)]"
      data-auto-route-recovery-kind={attention.kind}
      ref={cardRef}
      role={isPreparing ? "status" : "alert"}
      tabIndex={-1}
    >
      <h3 className="text-sm font-semibold" id="auto-route-attention-title">
        {t(copy.title)}
      </h3>
      <p className="mt-1 text-sm leading-6 text-[var(--foreground-muted)]">
        {t(copy.body)}
      </p>
      <AutoRouteRecoveryControls attention={attention} hasCloudModel={hasCloudModel} hasVerifiedLocalModel={hasVerifiedLocalModel} localLabel={localLabel} onChoice={onChoice} t={t} />
      <AutoRouteTechnicalDetails attention={attention} t={t} />
    </section>
  );
}
