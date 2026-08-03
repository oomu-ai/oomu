"use client";

import { useEffect, useRef, useState } from "react";
import {
  browserAutomationApi,
  type BrowserAutomationState,
  type BrowserSession,
  type BrowserSnapshot,
} from "./browserClient";

type Translate = (
  key: string,
  values?: Record<string, string | number>,
) => string;

export function BrowserTaskPanel({
  pollIntervalMs = 1_000,
  projectId,
  taskRunId,
  t,
}: {
  pollIntervalMs?: number;
  projectId: string;
  taskRunId: string;
  t: Translate;
}) {
  const [session, setSession] = useState<BrowserSession | null>(null);
  const [consent, setConsent] = useState(false);
  const [starting, setStarting] = useState(false);
  const [errorKey, setErrorKey] = useState("");
  const requestVersion = useRef(0);
  const controlsInFlight = useRef(0);
  const checksInFlight = useRef(0);
  const takeoverAfterCheck = useRef(false);
  const sessionId = session?.sessionId;

  useEffect(() => {
    if (!sessionId) return;
    const refresh = async () => {
      if (controlsInFlight.current > 0 || checksInFlight.current > 0) return;
      const version = ++requestVersion.current;
      try {
        const next = await browserAutomationApi.get(sessionId, taskRunId);
        if (requestVersion.current === version) setSession(next);
      } catch {
        // Keep an existing safety control visible through a transient refresh failure.
      }
    };
    if (pollIntervalMs <= 0) return;
    const timer = window.setInterval(() => { void refresh(); }, pollIntervalMs);
    return () => {
      window.clearInterval(timer);
      requestVersion.current += 1;
    };
  }, [pollIntervalMs, sessionId, taskRunId]);

  async function checkPage(current: BrowserSession) {
    const version = ++requestVersion.current;
    checksInFlight.current += 1;
    try {
      const result = await browserAutomationApi.action(
        current,
        { kind: "snapshot" },
        t("browser.check_page"),
      );
      if (!result.observation) throw new Error("browser_page_check_empty");
      const next = withSnapshot(current, result.observation);
      if (requestVersion.current !== version) return next;
      if (takeoverAfterCheck.current) {
        takeoverAfterCheck.current = false;
        const takeoverVersion = ++requestVersion.current;
        controlsInFlight.current += 1;
        try {
          const taken = await browserAutomationApi.control(
            next.sessionId,
            taskRunId,
            "takeover",
          );
          if (requestVersion.current === takeoverVersion) {
            setSession(taken);
            setErrorKey("");
          }
          return taken;
        } catch {
          if (requestVersion.current === takeoverVersion) {
            setSession(next);
            setErrorKey("browser.errors.control_failed");
          }
          return next;
        } finally {
          controlsInFlight.current = Math.max(0, controlsInFlight.current - 1);
        }
      }
      setSession(next);
      setErrorKey("");
      return next;
    } catch {
      if (requestVersion.current === version) {
        setErrorKey("browser.errors.page_check_failed");
      }
      return current;
    } finally {
      checksInFlight.current = Math.max(0, checksInFlight.current - 1);
    }
  }

  async function start() {
    setStarting(true);
    setErrorKey("");
    try {
      const started = await browserAutomationApi.start(taskRunId, projectId, consent);
      setSession(started);
      await checkPage(started);
    } catch {
      setErrorKey("browser.errors.start_failed");
    } finally {
      setStarting(false);
    }
  }

  async function control(kind: "pause" | "takeover" | "return") {
    if (!session) return;
    if (kind === "takeover" && session.state === "return_pending") {
      takeoverAfterCheck.current = true;
      setErrorKey("");
      if (checksInFlight.current === 0) void checkPage(session);
      return;
    }
    const version = ++requestVersion.current;
    controlsInFlight.current += 1;
    setErrorKey("");
    try {
      const next = await browserAutomationApi.control(
        session.sessionId,
        taskRunId,
        kind,
      );
      if (requestVersion.current !== version) return;
      setSession(next);
      if (kind === "return") await checkPage(next);
    } catch {
      if (requestVersion.current === version) {
        setErrorKey("browser.errors.control_failed");
      }
    } finally {
      controlsInFlight.current = Math.max(0, controlsInFlight.current - 1);
    }
  }

  if (!session) {
    return (
      <section className="rounded-[var(--radius-md)] border border-[var(--border-soft)] p-4">
        <div className="flex items-start gap-3">
          <BrowserIcon />
          <div className="min-w-0 flex-1">
            <h3 className="text-sm font-semibold">{t("browser.title")}</h3>
            <p className="mt-1 text-xs leading-5 text-[var(--foreground-muted)]">
              {t("browser.subtitle")}
            </p>
          </div>
        </div>
        <label className="mt-4 flex items-start gap-2 text-xs leading-5">
          <input
            checked={consent}
            onChange={(event) => setConsent(event.target.checked)}
            type="checkbox"
          />
          <span>{t("browser.policy_consent")}</span>
        </label>
        <button
          className="mt-3 rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-3 py-2 text-sm font-semibold text-[var(--inverse-foreground)] disabled:opacity-50"
          disabled={starting || !consent}
          onClick={() => void start()}
          type="button"
        >
          {t(starting ? "browser.starting" : "browser.start")}
        </button>
        {errorKey ? <BrowserError message={t(errorKey)} /> : null}
      </section>
    );
  }

  const state = knownState(session.state);
  const page = pageName(session.snapshot, t);
  const checkedAt = session.snapshot?.capturedAtMs ?? session.lastSnapshotAtMs;

  return (
    <section
      aria-label={t(browserHeadlineKey(state))}
      className="rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-3 shadow-sm"
    >
      <div className="flex flex-wrap items-center gap-3">
        <BrowserIcon />
        <div className="min-w-0 flex-1">
          <h3 className="text-sm font-semibold">
            {t(browserHeadlineKey(state))}
          </h3>
          <p aria-live="polite" className="mt-0.5 text-xs leading-5 text-[var(--foreground-muted)]">
            {t(browserActivityKey(state, Boolean(session.snapshot)), { page })}
          </p>
        </div>
        <BrowserControls onControl={control} state={state} t={t} />
      </div>

      {session.snapshot?.possiblePromptInjection ? (
        <p className="mt-3 rounded bg-[var(--warning-background)] p-2 text-xs font-medium text-[var(--warning)]">
          {t("browser.page_instruction_warning")}
        </p>
      ) : null}
      {session.snapshot?.protectedInterruption ? (
        <p className="mt-3 rounded bg-[var(--warning-background)] p-2 text-xs font-medium text-[var(--warning)]">
          {t("browser.protected_flow")}
        </p>
      ) : null}
      {errorKey ? <BrowserError message={t(errorKey)} /> : null}

      <details className="mt-3 border-t border-[var(--border-soft)] pt-2 text-xs">
        <summary className="cursor-pointer text-[var(--foreground-muted)]">
          {t("common.details")}
        </summary>
        <dl className="mt-2 grid grid-cols-[7rem_minmax(0,1fr)] gap-x-3 gap-y-1 rounded bg-[var(--background)] p-2">
          <dt className="text-[var(--foreground-muted)]">{t("browser.details.page")}</dt>
          <dd>{page}</dd>
          <dt className="text-[var(--foreground-muted)]">{t("browser.details.checked")}</dt>
          <dd>{checkedAt ? new Date(checkedAt).toLocaleString() : t("browser.details.not_checked")}</dd>
          <dt className="text-[var(--foreground-muted)]">{t("browser.details.status")}</dt>
          <dd>{t(browserStatusKey(state))}</dd>
        </dl>
      </details>
    </section>
  );
}

function BrowserControls({
  onControl,
  state,
  t,
}: {
  onControl: (control: "pause" | "takeover" | "return") => void;
  state: BrowserAutomationState;
  t: Translate;
}) {
  if (state === "takeover") {
    return (
      <TranslatedButton
        emphasized
        labelKey="browser.return_to_oomu"
        onClick={() => void onControl("return")}
        t={t}
      />
    );
  }
  if (state === "paused") {
    return (
      <TranslatedButton
        emphasized
        labelKey="browser.take_control"
        onClick={() => void onControl("takeover")}
        t={t}
      />
    );
  }
  if (state === "return_pending") {
    return (
      <TranslatedButton
        emphasized
        labelKey="browser.take_control"
        onClick={() => void onControl("takeover")}
        t={t}
      />
    );
  }
  if (state !== "automating") return null;
  return (
    <div className="flex shrink-0 items-center gap-2">
      <TranslatedButton
        labelKey="browser.pause"
        onClick={() => void onControl("pause")}
        t={t}
      />
      <TranslatedButton
        emphasized
        labelKey="browser.take_control"
        onClick={() => void onControl("takeover")}
        t={t}
      />
    </div>
  );
}

function TranslatedButton({
  emphasized = false,
  labelKey,
  onClick,
  t,
}: {
  emphasized?: boolean;
  labelKey: string;
  onClick: () => void;
  t: Translate;
}) {
  return (
    <button
      className={emphasized
        ? "rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-3 py-2 text-sm font-semibold text-[var(--inverse-foreground)]"
        : "rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-sm font-semibold"}
      onClick={onClick}
      type="button"
    >
      {t(labelKey)}
    </button>
  );
}

function BrowserIcon() {
  return (
    <span aria-hidden="true" className="grid h-9 w-9 shrink-0 place-items-center rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--background)]">
      <svg aria-hidden="true" className="h-5 w-5" fill="none" stroke="currentColor" strokeWidth="1.8" viewBox="0 0 24 24">
        <circle cx="12" cy="12" r="9" />
        <path d="M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18" />
      </svg>
    </span>
  );
}

function BrowserError({ message }: { message: string }) {
  return <p className="mt-3 text-xs font-medium text-[var(--warning)]" role="alert">{message}</p>;
}

function withSnapshot(session: BrowserSession, snapshot: BrowserSnapshot): BrowserSession {
  return {
    ...session,
    state: session.state === "return_pending" ? "automating" : session.state,
    documentGeneration: snapshot.documentGeneration,
    lastSnapshotAtMs: snapshot.capturedAtMs,
    snapshot,
  };
}

function pageName(snapshot: BrowserSnapshot | null, t: Translate) {
  const title = snapshot?.title.trim() ?? "";
  if (!title || title.length > 160 || /^https?:\/\//i.test(title)) {
    return t("browser.open_page");
  }
  return title;
}

function knownState(value: BrowserAutomationState): BrowserAutomationState {
  return [
    "automating",
    "paused",
    "takeover",
    "return_pending",
    "stopped",
    "closed",
  ].includes(value)
    ? value
    : "paused";
}

export function browserHeadlineKey(state: BrowserAutomationState) {
  return `browser.headline.${knownState(state)}`;
}

export function browserStatusKey(state: BrowserAutomationState) {
  return `browser.status.${knownState(state)}`;
}

export function browserActivityKey(
  state: BrowserAutomationState,
  hasPage: boolean,
) {
  const safeState = knownState(state);
  if (safeState === "automating") {
    return hasPage ? "browser.activity.working" : "browser.activity.checking";
  }
  return `browser.activity.${safeState}`;
}
