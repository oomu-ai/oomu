"use client";

import { useEffect, useRef, useState } from "react";
import { useI18n } from "@/context/I18nContext";
import {
  appControlApi,
  type AppControlActionKind,
  type AppControlControl,
  type AppControlIcon,
  type AppControlOutcomeStatus,
  type AppControlPauseReason,
  type AppControlSessionView,
  type AppControlState,
} from "./appControlClient";

const ACTIVE_STATES = new Set<AppControlState>([
  "observing",
  "running",
  "paused",
  "takeover",
  "return_pending",
]);

const ACTION_KEYS: Record<AppControlActionKind, string> = {
  focus: "app_control.activity.focus",
  press: "app_control.activity.press",
  select: "app_control.activity.select",
  type_text: "app_control.activity.type",
  invoke_menu: "app_control.activity.invoke_menu",
  scroll: "app_control.activity.scroll",
  drag_drop: "app_control.activity.drag_drop",
  choose_file: "app_control.activity.choose_file",
  apple_event: "app_control_actions.activate",
};

const PAUSE_KEYS: Record<AppControlPauseReason, string> = {
  user_input: "app_control.pause_reason.user_input",
  secure_field: "app_control.pause_reason.secure_field",
  ambiguous_target: "app_control.pause_reason.ambiguous_target",
  repeated_mismatch: "app_control.pause_reason.repeated_mismatch",
  unexpected_navigation: "app_control.pause_reason.unexpected_navigation",
  permission_changed: "app_control.pause_reason.permission_changed",
  hidden_window: "app_control.pause_reason.hidden_window",
  application_changed: "app_control.pause_reason.application_changed",
  driver_unavailable: "app_control.pause_reason.control_unavailable",
};

const OUTCOME_KEYS: Record<AppControlOutcomeStatus, string> = {
  verified: "app_control.outcome.verified",
  no_change: "app_control.outcome.no_change",
  failed: "app_control.outcome.failed",
  paused: "app_control.outcome.paused",
};

type Translate = (
  key: string,
  variables?: Record<string, string | number>,
) => string;

export function AppControlMonitor({
  pollIntervalMs = 1_000,
  taskRunId,
}: {
  pollIntervalMs?: number;
  taskRunId?: string;
}) {
  const { language, t } = useI18n();
  const [session, setSession] = useState<AppControlSessionView | null>(null);
  const [controlError, setControlError] = useState(false);
  const requestVersion = useRef(0);
  const controlsInFlight = useRef(0);

  useEffect(() => {
    const refresh = async () => {
      if (controlsInFlight.current > 0) return;
      const version = ++requestVersion.current;
      try {
        const next = await appControlApi.getStatus(taskRunId);
        if (requestVersion.current === version) setSession(next);
      } catch {
        // A transient refresh failure must not hide already-visible controls.
      }
    };
    const initial = window.setTimeout(() => { void refresh(); }, 0);
    const timer = pollIntervalMs > 0
      ? window.setInterval(() => { void refresh(); }, pollIntervalMs)
      : null;
    return () => {
      window.clearTimeout(initial);
      if (timer !== null) window.clearInterval(timer);
      requestVersion.current += 1;
    };
  }, [pollIntervalMs, taskRunId]);

  async function control(controlKind: AppControlControl) {
    if (!session) return;
    const previous = session;
    const version = ++requestVersion.current;
    controlsInFlight.current += 1;
    setControlError(false);
    if (controlKind === "return_to_oomu") {
      setSession({
        ...session,
        state: "return_pending",
        currentAction: null,
        pauseReason: null,
        canPause: true,
        canTakeControl: true,
        canReturnToOomu: false,
      });
    }
    try {
      const next = await appControlApi.control(
        session.sessionId,
        session.taskRunId,
        controlKind,
      );
      if (requestVersion.current === version) setSession(next);
    } catch {
      if (requestVersion.current === version) {
        setSession(previous);
        setControlError(true);
      }
    } finally {
      controlsInFlight.current = Math.max(0, controlsInFlight.current - 1);
    }
  }

  if (!session) return null;

  const appName = safeAppName(
    session.application?.name,
    t("app_control.unknown_app"),
  );
  const state = knownState(session.state);
  const headline = t(headlineKey(state), { app: appName });
  const activity = activityCopy(session, state, appName, t);
  const result = outcomeKey(session.lastOutcome?.status, state);

  return (
    <aside
      aria-label={t("app_control.title")}
      className="rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-3 shadow-sm"
      data-testid="app-control-monitor"
    >
      <div className="flex flex-wrap items-center gap-3">
        <AppIdentityIcon icon={knownIcon(session.application?.icon)} />
        <div className="min-w-0 flex-1">
          <p className="text-sm font-semibold text-[var(--foreground)]">{headline}</p>
          <p aria-live="polite" className="mt-0.5 text-xs leading-5 text-[var(--foreground-muted)]">
            {activity}
          </p>
        </div>
        <ControlButtons onControl={control} state={state} />
      </div>

      {controlError ? (
        <p className="mt-2 text-xs font-medium text-[var(--warning)]" role="alert">
          {t("app_control.control_failed")}
        </p>
      ) : null}

      <details className="mt-2 border-t border-[var(--border-soft)] pt-2 text-xs">
        <summary className="cursor-pointer text-[var(--foreground-muted)]">
          {t("common.details")}
        </summary>
        <dl className="mt-2 grid grid-cols-[7rem_minmax(0,1fr)] gap-x-3 gap-y-1 rounded bg-[var(--background)] p-2">
          <dt className="text-[var(--foreground-muted)]">{t("app_control.details.app")}</dt>
          <dd>{appName}</dd>
          <dt className="text-[var(--foreground-muted)]">{t("app_control.details.checked")}</dt>
          <dd>{new Date(session.updatedAtMs).toLocaleString(language)}</dd>
          <dt className="text-[var(--foreground-muted)]">{t("app_control.details.result")}</dt>
          <dd>{t(result)}</dd>
        </dl>
      </details>
    </aside>
  );
}

function ControlButtons({
  onControl,
  state,
}: {
  onControl: (control: AppControlControl) => void;
  state: AppControlState;
}) {
  const { t } = useI18n();
  if (state === "takeover") {
    return (
      <button
        className="rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-3 py-2 text-sm font-semibold text-[var(--inverse-foreground)]"
        onClick={() => void onControl("return_to_oomu")}
        type="button"
      >
        {t("app_control.return_to_oomu")}
      </button>
    );
  }
  if (!ACTIVE_STATES.has(state)) return null;
  return (
    <div className="flex shrink-0 items-center gap-2">
      <button
        className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-sm font-semibold"
        onClick={() => void onControl(state === "paused" ? "return_to_oomu" : "pause")}
        type="button"
      >
        {t(state === "paused" ? "app_control.continue" : "app_control.pause")}
      </button>
      <button
        className="rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-3 py-2 text-sm font-semibold text-[var(--inverse-foreground)]"
        onClick={() => void onControl("take_control")}
        type="button"
      >
        {t("app_control.take_control")}
      </button>
    </div>
  );
}

function AppIdentityIcon({ icon }: { icon: AppControlIcon }) {
  return (
    <span
      aria-hidden="true"
      className="grid h-9 w-9 shrink-0 place-items-center overflow-hidden rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--background)]"
    >
      <AppIcon icon={icon} />
    </span>
  );
}

function activityCopy(
  session: AppControlSessionView,
  state: AppControlState,
  app: string,
  t: Translate,
) {
  if (state === "takeover") return t("app_control.user_control_help");
  if (state === "observing") return t("app_control.preparing_help");
  if (state === "return_pending") return t("app_control.rechecking");
  if (state === "paused") return t(pauseReasonKey(session.pauseReason));
  if (state === "completed" || state === "stopped" || state === "failed") {
    return t(outcomeKey(session.lastOutcome?.status, state));
  }
  return t(actionKey(session.currentAction?.kind), { app });
}

export function actionKey(value?: AppControlActionKind | null) {
  return value && value in ACTION_KEYS
    ? ACTION_KEYS[value as AppControlActionKind]
    : "app_control.activity.unknown";
}

export function pauseReasonKey(value?: AppControlPauseReason | null) {
  return value && value in PAUSE_KEYS
    ? PAUSE_KEYS[value as AppControlPauseReason]
    : "app_control.pause_reason.unknown";
}

export function outcomeKey(
  value: AppControlOutcomeStatus | null | undefined,
  state: AppControlState,
) {
  if (state === "failed") return "app_control.outcome.failed";
  if (state === "stopped") return "app_control.outcome.stopped";
  if (value && value in OUTCOME_KEYS) return OUTCOME_KEYS[value as AppControlOutcomeStatus];
  if (state === "completed") return "app_control.outcome.completed";
  return "app_control.outcome.in_progress";
}

function knownState(value: AppControlState): AppControlState {
  return [
    "observing",
    "running",
    "paused",
    "takeover",
    "return_pending",
    "completed",
    "stopped",
    "failed",
  ].includes(value)
    ? value
    : "paused";
}

function headlineKey(state: AppControlState) {
  if (state === "paused") return "app_control.headline.paused";
  if (state === "takeover") return "app_control.headline.user_control";
  if (state === "return_pending") return "app_control.headline.rechecking";
  if (state === "completed") return "app_control.headline.completed";
  if (state === "stopped") return "app_control.headline.stopped";
  if (state === "failed") return "app_control.headline.failed";
  if (state === "observing") return "app_control.headline.preparing";
  return "app_control.headline.working";
}

function knownIcon(value?: AppControlIcon) {
  return [
    "finder",
    "preview",
    "mail",
    "calendar",
    "numbers",
    "keynote",
    "excel",
    "powerpoint",
    "generic",
  ].includes(value ?? "")
    ? (value as AppControlIcon)
    : "generic";
}

function safeAppName(value: string | undefined, fallback: string) {
  const name = value?.trim() ?? "";
  if (
    !name
    || name.length > 80
    || /^[a-z0-9-]+(?:\.[a-z0-9-]+){2,}$/i.test(name)
  ) {
    return fallback;
  }
  return name;
}

function AppIcon({ icon }: { icon: AppControlIcon }) {
  if (icon === "mail") {
    return <svg aria-hidden="true" className="h-5 w-5" fill="none" stroke="currentColor" strokeWidth="1.8" viewBox="0 0 24 24"><rect height="14" rx="2" width="18" x="3" y="5" /><path d="m4 7 8 6 8-6" /></svg>;
  }
  if (icon === "calendar") {
    return <svg aria-hidden="true" className="h-5 w-5" fill="none" stroke="currentColor" strokeWidth="1.8" viewBox="0 0 24 24"><rect height="17" rx="2" width="18" x="3" y="4" /><path d="M7 2v4m10-4v4M3 9h18" /></svg>;
  }
  if (icon === "finder") {
    return <svg aria-hidden="true" className="h-5 w-5" fill="none" stroke="currentColor" strokeWidth="1.8" viewBox="0 0 24 24"><path d="M3 7h7l2 2h9v10H3z" /><path d="M3 7V5h7l2 2" /></svg>;
  }
  if (icon === "numbers" || icon === "excel") {
    return <svg aria-hidden="true" className="h-5 w-5" fill="none" stroke="currentColor" strokeWidth="1.8" viewBox="0 0 24 24"><rect height="18" rx="2" width="18" x="3" y="3" /><path d="M3 9h18M9 3v18m6-12v12" /></svg>;
  }
  if (icon === "keynote" || icon === "powerpoint") {
    return <svg aria-hidden="true" className="h-5 w-5" fill="none" stroke="currentColor" strokeWidth="1.8" viewBox="0 0 24 24"><rect height="13" rx="2" width="18" x="3" y="4" /><path d="M12 17v4m-4 0h8M7 13l3-3 2 2 3-4 2 5" /></svg>;
  }
  if (icon === "preview") {
    return <svg aria-hidden="true" className="h-5 w-5" fill="none" stroke="currentColor" strokeWidth="1.8" viewBox="0 0 24 24"><path d="M6 3h9l3 3v15H6z" /><path d="M15 3v4h4M9 12h6m-6 4h6" /></svg>;
  }
  return <svg aria-hidden="true" className="h-5 w-5" fill="none" stroke="currentColor" strokeWidth="1.8" viewBox="0 0 24 24"><rect height="16" rx="3" width="16" x="4" y="4" /><path d="M8 9h8M8 13h5" /></svg>;
}
