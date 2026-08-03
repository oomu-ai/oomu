"use client";

import { useEffect, useId, useMemo, useState } from "react";
import { invoke, isTauriRuntime } from "@/lib/invoke";
import { useI18n } from "@/context/I18nContext";

type ActuationLease = {
  sessionId: string;
  expiresAtMs: number;
  maxSteps: number;
  currentSteps: number;
  isActive: boolean;
};

type ActuationLeaseStatus = {
  lease: ActuationLease | null;
  active: boolean;
  nowMs: number;
  remainingMs: number;
  remainingSteps: number;
  reason: string | null;
};

type ActuationLeaseDecayEvent = {
  status: ActuationLeaseStatus;
  reason: string;
  operation?: string | null;
  sessionId: string;
  reviewPreview?: string | null;
};

const DURATION_OPTIONS = [
  { label: "5m", value: 5 * 60 * 1000 },
  { label: "15m", value: 15 * 60 * 1000 },
];

const ACTUATION_OPERATION_CLASSES = [
  "airlock_export",
  "codebase_compile",
  "codebase_patch",
  "delete_file",
  "document_index",
  "filesystem_write",
  "registered_task_tool",
  "shell_command",
  "telemetry_archive",
];

export function OperationsControl() {
  const { language } = useI18n();
  const componentSessionId = useId().replaceAll(":", "");
  const sessionId = `operations-dashboard-${componentSessionId}`;
  const [status, setStatus] = useState<ActuationLeaseStatus | null>(null);
  const [durationMs, setDurationMs] = useState(DURATION_OPTIONS[1].value);
  const [maxSteps, setMaxSteps] = useState(5);
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [decayEvent, setDecayEvent] = useState<ActuationLeaseDecayEvent | null>(null);

  const lease = status?.lease ?? null;
  const active = Boolean(status?.active && lease?.isActive);
  const remainingLabel = status ? formatRemaining(status.remainingMs) : "—";
  const stepLabel = status ? (lease ? `${lease.currentSteps}/${lease.maxSteps}` : "0/0") : "—";
  const stepPercent = lease?.maxSteps
    ? Math.min(100, (lease.currentSteps / lease.maxSteps) * 100)
    : 0;
  const timePercent = lease && status
    ? Math.max(0, Math.min(100, (status.remainingMs / durationMs) * 100))
    : 0;
  const reasonLabel = useMemo(() => humanizeReason(status?.reason), [status?.reason]);

  async function refreshStatus() {
    try {
      const result = await invoke<ActuationLeaseStatus>("get_actuation_lease_status");
      setStatus(result);
      setError(null);
    } catch (nextError) {
      setStatus(null);
      setError(describeError(nextError));
    }
  }

  async function grantLease() {
    setIsSaving(true);
    setError(null);
    try {
      const authority = await invoke<{ proofId: string }>("request_native_authority", {
        request: {
          sessionId,
          operationClasses: ACTUATION_OPERATION_CLASSES,
          scopes: [`actuation-session:${sessionId}`],
          maxSteps,
          persistence: "session_gated",
          locale: language,
        },
      });
      const nextStatus = await invoke<ActuationLeaseStatus>("grant_actuation_lease", {
        request: {
          sessionId,
          durationMs,
          maxSteps,
          authorityProofId: authority.proofId,
          operationClasses: ACTUATION_OPERATION_CLASSES,
        },
      });
      setDecayEvent(null);
      setStatus(nextStatus);
    } catch (nextError) {
      setError(describeError(nextError));
    } finally {
      setIsSaving(false);
    }
  }

  async function revokeLease() {
    setIsSaving(true);
    setError(null);
    try {
      const nextStatus = await invoke<ActuationLeaseStatus>("revoke_actuation_lease", {
        request: {
          sessionId,
          reason: "manual_revocation",
        },
      });
      setStatus(nextStatus);
    } catch (nextError) {
      setError(describeError(nextError));
    } finally {
      setIsSaving(false);
    }
  }

  useEffect(() => {
    const poll = () => void refreshStatus();
    const initial = window.setTimeout(poll, 0);
    const interval = window.setInterval(poll, 1000);
    return () => {
      window.clearTimeout(initial);
      window.clearInterval(interval);
    };
  }, []);

  useEffect(() => {
    if (!isTauriRuntime) return;

    let disposed = false;
    const unlisteners: Array<() => void> = [];
    void (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        const unlistenUpdated = await listen<ActuationLeaseStatus>(
          "actuation-lease-updated",
          (event) => {
            if (disposed) return;
            setDecayEvent(null);
            setStatus(event.payload);
          },
        );
        if (disposed) {
          unlistenUpdated();
          return;
        }
        unlisteners.push(unlistenUpdated);

        const unlistenDecayed = await listen<ActuationLeaseDecayEvent>(
          "actuation-lease-decayed",
          (event) => {
            if (disposed) return;
            setDecayEvent(event.payload);
            setStatus(event.payload.status);
          },
        );
        if (disposed) {
          unlistenDecayed();
          return;
        }
        unlisteners.push(unlistenDecayed);
      } catch (nextError) {
        setError(describeError(nextError));
      }
    })();

    return () => {
      disposed = true;
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, []);

  return (
    <section className="rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--background)] p-5">
      <div className="flex flex-col gap-4 lg:flex-row lg:items-start lg:justify-between">
        <div>
          <div className="flex items-center gap-2">
            <PowerIcon active={active} />
            <h2 className="text-sm font-semibold text-[var(--foreground)]">Autopilot</h2>
          </div>
          <p className="mt-1.5 text-sm leading-6 text-[var(--foreground-muted)]">
            {status
              ? active
                ? `${remainingLabel} remaining`
                : reasonLabel ?? "Manual"
              : error
                ? "Unavailable"
                : "Checking native status..."}
          </p>
        </div>

        <button
          className={`inline-flex shrink-0 items-center justify-center gap-2 rounded-[var(--radius-sm)] px-3 py-2 text-sm font-semibold transition-colors disabled:cursor-wait disabled:opacity-60 ${
            active
              ? "border border-[var(--border-strong)] bg-[var(--background)] text-[var(--foreground)] hover:bg-[var(--fill-hover)]"
              : "bg-[var(--inverse-background)] text-[var(--inverse-foreground)] hover:bg-[var(--accent-hover)]"
          }`}
          disabled={isSaving || !status}
          onClick={() => void (active ? revokeLease() : grantLease())}
          title={status ? (active ? "Stop Autopilot" : "Start Autopilot") : "Autopilot unavailable"}
          type="button"
        >
          <PowerIcon active={!active} />
          {isSaving ? "Saving..." : status ? (active ? "Stop" : "Start") : "Unavailable"}
        </button>
      </div>

      <div className="mt-5 grid gap-5 lg:grid-cols-[1.1fr_0.9fr]">
        <div className="flex flex-col gap-4">
          <div>
            <div className="mb-2 flex items-center justify-between gap-3 text-xs font-medium">
              <span className="text-[var(--foreground-muted)]">Time</span>
              <span className="font-mono text-[var(--foreground)]">{remainingLabel}</span>
            </div>
            <div className="h-2 overflow-hidden rounded-full bg-[var(--fill)]">
              <div
                className="h-full bg-[var(--success)] transition-[width]"
                style={{ width: `${timePercent}%` }}
              />
            </div>
          </div>

          <div>
            <div className="mb-2 flex items-center justify-between gap-3 text-xs font-medium">
              <span className="text-[var(--foreground-muted)]">Steps</span>
              <span className="font-mono text-[var(--foreground)]">{stepLabel}</span>
            </div>
            <div className="h-2 overflow-hidden rounded-full bg-[var(--fill)]">
              <div
                className="h-full bg-[var(--warning)] transition-[width]"
                style={{ width: `${stepPercent}%` }}
              />
            </div>
          </div>
        </div>

        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-1">
          <div>
            <label className="text-xs font-semibold uppercase tracking-[0.14em] text-[var(--foreground-subtle)]">
              Duration
            </label>
            <div className="mt-2 inline-flex rounded-[var(--radius-sm)] bg-[var(--accent-background)] p-0.5">
              {DURATION_OPTIONS.map((option) => (
                <button
                  aria-pressed={durationMs === option.value}
                  className={`rounded-[var(--radius-sm)] px-3 py-1.5 text-xs font-semibold transition-colors ${
                    durationMs === option.value
                      ? "bg-[var(--background)] text-[var(--foreground)] shadow-[var(--shadow-card)]"
                      : "text-[var(--foreground-muted)] hover:text-[var(--foreground)]"
                  }`}
                  disabled={!status || active || isSaving}
                  key={option.value}
                  onClick={() => setDurationMs(option.value)}
                  type="button"
                >
                  {option.label}
                </button>
              ))}
            </div>
          </div>

          <div>
            <label
              className="text-xs font-semibold uppercase tracking-[0.14em] text-[var(--foreground-subtle)]"
              htmlFor="operations-control-max-steps"
            >
              Step Limit
            </label>
            <input
              className="mt-2 w-28 rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--background)] px-3 py-2 font-mono text-sm text-[var(--foreground)] outline-none transition-colors focus:border-[var(--border-strong)] disabled:opacity-60"
              disabled={!status || active || isSaving}
              id="operations-control-max-steps"
              max={50}
              min={1}
              onChange={(event) => setMaxSteps(clampStepLimit(event.target.value))}
              type="number"
              value={maxSteps}
            />
          </div>
        </div>
      </div>

      {decayEvent?.reviewPreview ? (
        <div className="mt-5 border-t border-[var(--border-soft)] pt-4">
          <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
            <p className="text-xs font-semibold uppercase tracking-[0.14em] text-[var(--foreground-subtle)]">
              Review Diff
            </p>
            <span className="text-xs text-[var(--foreground-muted)]">
              {decayEvent.operation ?? decayEvent.reason}
            </span>
          </div>
          <pre className="max-h-56 overflow-auto rounded-[var(--radius-base)] border border-[var(--border-soft)] bg-[var(--fill)] p-3 font-mono text-xs leading-5 text-[var(--foreground)]">
            {decayEvent.reviewPreview}
          </pre>
        </div>
      ) : null}

      {error ? (
        <p className="mt-4 text-xs leading-5 text-[var(--destructive)]" aria-live="polite">
          {error}
        </p>
      ) : null}
    </section>
  );
}

// Turn a backend reason enum into a readable phrase instead of leaking the raw
// snake_case token ("manual_revocation" → "Stopped manually"). Unknown reasons
// fall back to a title-cased spacing rather than a bare lowercase fragment.
function humanizeReason(reason: string | null | undefined) {
  if (!reason) return null;
  const known: Record<string, string> = {
    manual_revocation: "Stopped manually",
    expired: "Session expired",
    step_limit_reached: "Step limit reached",
    time_limit_reached: "Time limit reached",
    no_lease: "Manual",
  };
  return (
    known[reason] ??
    reason.replaceAll("_", " ").replace(/\b\w/g, (letter) => letter.toUpperCase())
  );
}

function clampStepLimit(value: string) {
  const parsed = Number.parseInt(value, 10);
  if (!Number.isFinite(parsed)) return 1;
  return Math.min(50, Math.max(1, parsed));
}

function formatRemaining(ms: number) {
  const totalSeconds = Math.max(0, Math.ceil(ms / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

function describeError(error: unknown) {
  if (error && typeof error === "object" && "message" in error) {
    const message = (error as { message?: unknown }).message;
    if (typeof message === "string") return message;
  }
  return String(error);
}

function PowerIcon({ active }: { active: boolean }) {
  return (
    <svg
      aria-hidden="true"
      className={`h-4 w-4 ${active ? "text-[var(--success)]" : "text-[var(--foreground-muted)]"}`}
      fill="none"
      stroke="currentColor"
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth="1.8"
      viewBox="0 0 24 24"
    >
      <path d="M12 2v10" />
      <path d="M18.4 6.6a9 9 0 1 1-12.8 0" />
    </svg>
  );
}
