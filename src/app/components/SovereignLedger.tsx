"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@/lib/invoke";
import { useI18n } from "@/context/I18nContext";

type LedgerStats = {
  totalLocalTurns?: number;
  total_local_turns?: number;
  totalCloudTurns?: number;
  total_cloud_turns?: number;
  ratioOnDevice?: number;
  ratio_on_device?: number;
  estimatedApiSavings?: number;
  estimated_api_savings?: number;
  dataEgressProtectedMb?: number;
  data_egress_protected_mb?: number;
  protectedInputTokens?: number;
  protected_input_tokens?: number;
  protectedOutputTokens?: number;
  protected_output_tokens?: number;
};

type NormalizedLedgerStats = {
  totalLocalTurns: number;
  totalCloudTurns: number;
  ratioOnDevice: number;
  dataEgressProtectedMb: number;
  protectedInputTokens: number;
  protectedOutputTokens: number;
};

// Defaults mirror the backend's assumption (GPT-class API pricing). They are a
// starting point, not a fact — a reseller or a different model changes them, so
// the user can override and we recompute savings from the raw token counts.
const DEFAULT_INPUT_USD_PER_MILLION = 1.25;
const DEFAULT_OUTPUT_USD_PER_MILLION = 5.0;
const PRICING_STORAGE_KEY = "oomu.ledger.pricing.v1";
const DAY_MS = 24 * 60 * 60 * 1000;
const LIVE_REFRESH_INTERVAL_MS = 15_000;
const ACTIVITY_SETTLE_MS = 1_500;
const TERMINAL_ACTIVITY_STATUSES = new Set([
  "cancelled",
  "completed",
  "failed",
  "halted",
  "skipped",
  "success",
  "verified",
]);

const LEDGER_PERIODS = [
  { id: "today", labelKey: "ledger.period_today" },
  { id: "last7", labelKey: "ledger.period_last_7" },
  { id: "last30", labelKey: "ledger.period_last_30" },
  { id: "last90", labelKey: "ledger.period_last_90" },
  { id: "last6Months", labelKey: "ledger.period_last_6_months" },
  { id: "last12Months", labelKey: "ledger.period_last_12_months" },
] as const;

type LedgerPeriodId = (typeof LEDGER_PERIODS)[number]["id"];
const DEFAULT_LEDGER_PERIOD: LedgerPeriodId = "last30";

type LedgerPricing = {
  inputPerMillion: number;
  outputPerMillion: number;
};

function normalizeLedgerStats(stats: LedgerStats): NormalizedLedgerStats {
  return {
    totalLocalTurns: stats.totalLocalTurns ?? stats.total_local_turns ?? 0,
    totalCloudTurns: stats.totalCloudTurns ?? stats.total_cloud_turns ?? 0,
    ratioOnDevice: stats.ratioOnDevice ?? stats.ratio_on_device ?? 0,
    dataEgressProtectedMb: stats.dataEgressProtectedMb ?? stats.data_egress_protected_mb ?? 0,
    protectedInputTokens: stats.protectedInputTokens ?? stats.protected_input_tokens ?? 0,
    protectedOutputTokens: stats.protectedOutputTokens ?? stats.protected_output_tokens ?? 0,
  };
}

function loadPricing(): LedgerPricing {
  if (typeof window === "undefined") {
    return { inputPerMillion: DEFAULT_INPUT_USD_PER_MILLION, outputPerMillion: DEFAULT_OUTPUT_USD_PER_MILLION };
  }
  try {
    const raw = window.localStorage.getItem(PRICING_STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as Partial<LedgerPricing>;
      return {
        inputPerMillion: clampPrice(parsed.inputPerMillion, DEFAULT_INPUT_USD_PER_MILLION),
        outputPerMillion: clampPrice(parsed.outputPerMillion, DEFAULT_OUTPUT_USD_PER_MILLION),
      };
    }
  } catch {
    // Corrupt or unavailable storage falls back to defaults.
  }
  return { inputPerMillion: DEFAULT_INPUT_USD_PER_MILLION, outputPerMillion: DEFAULT_OUTPUT_USD_PER_MILLION };
}

function clampPrice(value: unknown, fallback: number) {
  return typeof value === "number" && Number.isFinite(value) && value >= 0 ? value : fallback;
}

function estimateSavings(stats: NormalizedLedgerStats, pricing: LedgerPricing) {
  return (
    (stats.protectedInputTokens / 1_000_000) * pricing.inputPerMillion +
    (stats.protectedOutputTokens / 1_000_000) * pricing.outputPerMillion
  );
}

function formatApproxCurrency(value: number, locale: string) {
  const safeValue = Number.isFinite(value) ? Math.max(0, value) : 0;
  const cents = new Intl.NumberFormat(locale, {
    currency: "USD",
    maximumFractionDigits: 2,
    minimumFractionDigits: 2,
    style: "currency",
  });
  if (safeValue > 0 && safeValue < 0.005) {
    return `< ${cents.format(0.01)}`;
  }
  const formatted =
    safeValue < 100
      ? cents.format(safeValue)
      : new Intl.NumberFormat(locale, {
          currency: "USD",
          maximumFractionDigits: 0,
          style: "currency",
        }).format(safeValue);
  return safeValue > 0 ? `≈ ${formatted}` : formatted;
}

function formatNumber(value: number, locale: string) {
  return new Intl.NumberFormat(locale).format(Math.max(0, Math.round(value)));
}

function formatTime(value: number, locale: string) {
  return new Intl.DateTimeFormat(locale, { hour: "2-digit", minute: "2-digit" }).format(value);
}

type LedgerActivityEvent = {
  payload?: {
    isFinal?: boolean;
    is_final?: boolean;
    status?: string;
  };
};

type StatsRequestOptions = {
  foreground: boolean;
  surfaceError: boolean;
};

function ledgerPeriodSinceMs(period: LedgerPeriodId) {
  const now = new Date();
  switch (period) {
    case "today":
      return new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
    case "last7":
      return now.getTime() - 7 * DAY_MS;
    case "last30":
      return now.getTime() - 30 * DAY_MS;
    case "last90":
      return now.getTime() - 90 * DAY_MS;
    case "last6Months": {
      const start = new Date(now);
      start.setMonth(start.getMonth() - 6);
      return start.getTime();
    }
    case "last12Months": {
      const start = new Date(now);
      start.setFullYear(start.getFullYear() - 1);
      return start.getTime();
    }
  }
}

export function SovereignLedger() {
  const { language, t } = useI18n();
  const tRef = useRef(t);
  const [stats, setStats] = useState<NormalizedLedgerStats | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [isResetting, setIsResetting] = useState(false);
  const [isResetDialogOpen, setIsResetDialogOpen] = useState(false);
  const [error, setError] = useState("");
  const [updatedAt, setUpdatedAt] = useState<number | null>(null);
  const [selectedPeriod, setSelectedPeriod] = useState<LedgerPeriodId>(DEFAULT_LEDGER_PERIOD);
  const [pricing, setPricing] = useState<LedgerPricing>(() => loadPricing());
  const mountedRef = useRef(false);
  const requestSequenceRef = useRef(0);
  const foregroundRefreshRef = useRef(true);

  useEffect(() => {
    tRef.current = t;
  }, [t]);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      requestSequenceRef.current += 1;
    };
  }, []);

  const fetchStats = useCallback(async (period: LedgerPeriodId) => {
    const response = await invoke<LedgerStats>("get_sovereign_ledger_stats", {
      sinceMs: ledgerPeriodSinceMs(period),
    });
    return normalizeLedgerStats(response);
  }, []);

  const requestStats = useCallback(
    async (period: LedgerPeriodId, options: StatsRequestOptions) => {
      if (options.foreground) {
        foregroundRefreshRef.current = true;
      }
      const requestSequence = ++requestSequenceRef.current;
      try {
        const next = await fetchStats(period);
        if (!mountedRef.current || requestSequence !== requestSequenceRef.current) {
          return false;
        }
        setStats(next);
        setUpdatedAt(Date.now());
        setError("");
        return true;
      } catch (err) {
        if (
          mountedRef.current &&
          requestSequence === requestSequenceRef.current &&
          options.surfaceError
        ) {
          setError(err instanceof Error ? err.message : tRef.current("ledger.error"));
        }
        return false;
      } finally {
        if (
          mountedRef.current &&
          requestSequence === requestSequenceRef.current &&
          options.foreground
        ) {
          foregroundRefreshRef.current = false;
          setIsLoading(false);
        }
      }
    },
    [fetchStats],
  );

  const beginForegroundRefresh = useCallback(() => {
    foregroundRefreshRef.current = true;
    requestSequenceRef.current += 1;
    setIsLoading(true);
    setError("");
  }, []);

  const loadStats = useCallback(async () => {
    beginForegroundRefresh();
    await requestStats(selectedPeriod, { foreground: true, surfaceError: true });
  }, [beginForegroundRefresh, requestStats, selectedPeriod]);

  // Initial load: state is only touched in the async continuation (never
  // synchronously in the effect body); request sequencing rejects late results.
  useEffect(() => {
    foregroundRefreshRef.current = true;
    const requestSequence = ++requestSequenceRef.current;
    fetchStats(selectedPeriod)
      .then((next) => {
        if (mountedRef.current && requestSequence === requestSequenceRef.current) {
          setStats(next);
          setUpdatedAt(Date.now());
          setError("");
        }
      })
      .catch((err) => {
        if (mountedRef.current && requestSequence === requestSequenceRef.current) {
          setError(err instanceof Error ? err.message : tRef.current("ledger.error"));
        }
      })
      .finally(() => {
        if (mountedRef.current && requestSequence === requestSequenceRef.current) {
          foregroundRefreshRef.current = false;
          setIsLoading(false);
        }
      });
  }, [fetchStats, selectedPeriod]);

  useEffect(() => {
    let cancelled = false;
    let activityTimer: number | null = null;
    let refreshInFlight = false;
    const unlisteners: Array<() => void> = [];

    const refreshInBackground = async () => {
      if (
        cancelled ||
        refreshInFlight ||
        foregroundRefreshRef.current ||
        document.visibilityState === "hidden"
      ) {
        return;
      }
      refreshInFlight = true;
      try {
        await requestStats(selectedPeriod, { foreground: false, surfaceError: false });
      } finally {
        refreshInFlight = false;
      }
    };

    const queueActivityRefresh = () => {
      if (cancelled) {
        return;
      }
      if (activityTimer !== null) {
        window.clearTimeout(activityTimer);
      }
      activityTimer = window.setTimeout(() => {
        activityTimer = null;
        void refreshInBackground();
      }, ACTIVITY_SETTLE_MS);
    };

    const terminalActivity = (event: LedgerActivityEvent) => {
      const status = event.payload?.status?.toLowerCase();
      if (status && TERMINAL_ACTIVITY_STATUSES.has(status)) {
        queueActivityRefresh();
      }
    };

    async function subscribeToLedgerActivity() {
      const registered: Array<() => void> = [];
      try {
        const { listen } = await import("@tauri-apps/api/event");
        registered.push(await listen("chat://token", queueActivityRefresh));
        registered.push(
          await listen<LedgerActivityEvent["payload"]>("token-stream", (event) => {
            if (event.payload?.isFinal || event.payload?.is_final) {
              queueActivityRefresh();
            }
          }),
        );
        registered.push(await listen("taskflow://progress", terminalActivity));
        registered.push(await listen("vwa://progress", terminalActivity));
      } catch {
        // The bounded poll below remains authoritative in browser previews and
        // older shells where native event subscriptions are unavailable.
      } finally {
        if (cancelled) {
          registered.forEach((unlisten) => unlisten());
        } else {
          unlisteners.push(...registered);
        }
      }
    }

    const handleVisibilityChange = () => {
      if (document.visibilityState === "visible") {
        void refreshInBackground();
      }
    };

    void subscribeToLedgerActivity();
    document.addEventListener("visibilitychange", handleVisibilityChange);
    const refreshInterval = window.setInterval(
      () => void refreshInBackground(),
      LIVE_REFRESH_INTERVAL_MS,
    );

    return () => {
      cancelled = true;
      if (activityTimer !== null) {
        window.clearTimeout(activityTimer);
      }
      window.clearInterval(refreshInterval);
      document.removeEventListener("visibilitychange", handleVisibilityChange);
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, [requestStats, selectedPeriod]);

  const changePeriod = useCallback((nextPeriod: LedgerPeriodId) => {
    beginForegroundRefresh();
    setSelectedPeriod(nextPeriod);
  }, [beginForegroundRefresh]);

  const resetLedger = useCallback(async () => {
    beginForegroundRefresh();
    setIsResetting(true);
    const resetSequence = requestSequenceRef.current;
    let refreshStarted = false;
    try {
      await invoke<void>("reset_sovereign_ledger_stats");
      if (!mountedRef.current || resetSequence !== requestSequenceRef.current) {
        return;
      }
      refreshStarted = true;
      const refreshed = await requestStats(selectedPeriod, {
        foreground: true,
        surfaceError: true,
      });
      if (refreshed) {
        setIsResetDialogOpen(false);
      }
    } catch (err) {
      if (mountedRef.current && resetSequence === requestSequenceRef.current) {
        setError(err instanceof Error ? err.message : tRef.current("ledger.error"));
      }
    } finally {
      if (mountedRef.current) {
        setIsResetting(false);
      }
      if (
        !refreshStarted &&
        mountedRef.current &&
        resetSequence === requestSequenceRef.current
      ) {
        foregroundRefreshRef.current = false;
        setIsLoading(false);
      }
    }
  }, [beginForegroundRefresh, requestStats, selectedPeriod]);

  const updatePricing = useCallback((next: LedgerPricing) => {
    setPricing(next);
    try {
      window.localStorage.setItem(PRICING_STORAGE_KEY, JSON.stringify(next));
    } catch {
      // Persistence is best-effort; the session value still applies.
    }
  }, []);

  const totals = useMemo(() => {
    const local = stats?.totalLocalTurns ?? 0;
    const cloud = stats?.totalCloudTurns ?? 0;
    return {
      local,
      cloud,
      total: local + cloud,
      ratio: Math.max(0, Math.min(100, stats?.ratioOnDevice ?? 0)),
    };
  }, [stats]);

  const savings = stats ? estimateSavings(stats, pricing) : 0;
  const isCustomPricing =
    pricing.inputPerMillion !== DEFAULT_INPUT_USD_PER_MILLION ||
    pricing.outputPerMillion !== DEFAULT_OUTPUT_USD_PER_MILLION;
  const hasActivity = totals.total > 0;

  return (
    <main className="min-h-full bg-[var(--background)] text-[var(--foreground)]">
      <div className="mx-auto flex w-full max-w-6xl flex-col gap-6 px-8 py-6">
        <header className="flex flex-col gap-4 border-b border-[var(--border-soft)] pb-5 md:flex-row md:items-end md:justify-between">
          <div>
            <h1 className="text-2xl font-bold text-[var(--foreground)]">{t("ledger.title")}</h1>
            <p className="mt-2 max-w-2xl text-sm leading-6 text-[var(--foreground-muted)]">
              {t("ledger.subtitle")}
            </p>
          </div>
          <div className="flex flex-wrap items-center gap-2 md:justify-end">
            <label className="sr-only" htmlFor="ledger-period">
              {t("ledger.period_label")}
            </label>
            <select
              className="h-9 rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 text-[13px] font-semibold text-[var(--foreground)] outline-none transition-colors hover:bg-[var(--fill-hover)] focus:border-[var(--accent)] disabled:cursor-not-allowed disabled:opacity-50"
              disabled={isLoading || isResetting}
              id="ledger-period"
              onChange={(event) => changePeriod(event.target.value as LedgerPeriodId)}
              value={selectedPeriod}
            >
              {LEDGER_PERIODS.map((period) => (
                <option key={period.id} value={period.id}>
                  {t(period.labelKey)}
                </option>
              ))}
            </select>
            <button
              className="inline-flex h-9 items-center justify-center rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 text-[13px] font-semibold text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)] disabled:cursor-not-allowed disabled:opacity-50"
              disabled={isLoading || isResetting}
              onClick={() => void loadStats()}
              type="button"
            >
              {isLoading ? t("ledger.refreshing") : t("ledger.refresh")}
            </button>
            <button
              className="inline-flex h-9 items-center justify-center rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 text-[13px] font-semibold text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)] disabled:cursor-not-allowed disabled:opacity-50"
              disabled={isLoading || isResetting}
              onClick={() => setIsResetDialogOpen(true)}
              type="button"
            >
              {isResetting ? t("ledger.resetting") : t("ledger.reset")}
            </button>
            <span className="min-w-32 text-right text-[13px] font-medium text-[var(--foreground-subtle)]">
              {updatedAt
                ? t("ledger.updated", { time: formatTime(updatedAt, language) })
                : t("ledger.not_updated")}
            </span>
          </div>
        </header>

        {isResetDialogOpen && (
          <div className="fixed inset-0 z-50 grid place-items-center bg-black/40 p-4">
            <section
              aria-labelledby="ledger-reset-title"
              aria-modal="true"
              className="w-full max-w-md rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--background)] p-5 shadow-xl"
              role="dialog"
            >
              <h2 id="ledger-reset-title" className="text-base font-semibold text-[var(--foreground)]">
                {t("ledger.reset_dialog_title")}
              </h2>
              <p className="mt-3 text-sm leading-6 text-[var(--foreground-muted)]">
                {t("ledger.reset_dialog_body")}
              </p>
              <div className="mt-5 flex flex-wrap justify-end gap-2">
                <button
                  className="inline-flex h-9 items-center justify-center rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 text-[13px] font-semibold text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)] disabled:cursor-not-allowed disabled:opacity-50"
                  disabled={isResetting}
                  onClick={() => setIsResetDialogOpen(false)}
                  type="button"
                >
                  {t("common.cancel")}
                </button>
                <button
                  className="inline-flex h-9 items-center justify-center rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 text-[13px] font-semibold text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)] disabled:cursor-not-allowed disabled:opacity-50"
                  disabled={isResetting}
                  onClick={() => void resetLedger()}
                  type="button"
                >
                  {isResetting ? t("ledger.resetting") : t("ledger.reset_confirm_action")}
                </button>
              </div>
            </section>
          </div>
        )}

        {error && (
          <section className="rounded-[var(--radius-sm)] border border-[var(--destructive)] bg-[var(--destructive-background)] px-4 py-3 text-sm font-medium text-[var(--destructive)]">
            {error}
          </section>
        )}

        {isLoading && !stats ? (
          <section className="grid min-h-64 place-items-center rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--accent-background)]">
            <div className="flex items-center gap-2 text-sm font-semibold text-[var(--foreground-muted)]">
              <span className="h-3 w-3 animate-spin rounded-full border-2 border-[var(--accent)] border-t-transparent" />
              {t("ledger.loading")}
            </div>
          </section>
        ) : stats && !hasActivity ? (
          <section className="flex min-h-64 flex-col items-center justify-center gap-3 rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--accent-background)] px-6 py-12 text-center">
            <span
              aria-hidden="true"
              className="flex h-12 w-12 items-center justify-center rounded-full bg-[var(--route-local-background)] text-[var(--route-local)]"
            >
              <svg className="h-6 w-6" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.8" viewBox="0 0 24 24">
                <path d="M4 19V5" />
                <path d="M4 19h16" />
                <path d="M8 16V9" />
                <path d="M12 16V7" />
                <path d="M16 16v-5" />
              </svg>
            </span>
            <h2 className="text-base font-semibold text-[var(--foreground)]">{t("ledger.empty_title")}</h2>
            <p className="max-w-md text-sm leading-6 text-[var(--foreground-muted)]">{t("ledger.empty_body")}</p>
          </section>
        ) : stats ? (
          <>
            <section className="grid grid-cols-1 gap-4 md:grid-cols-3">
              {/* The two principled metrics lead; the money estimate is demoted
                  to last and rendered quietly — it is the softest claim here. */}
              <StatCard
                accentClass="text-[var(--accent)]"
                detail={t("ledger.ratio_detail", {
                  local: formatNumber(totals.local, language),
                  total: formatNumber(totals.total, language),
                })}
                label={t("ledger.ratio_label")}
                value={`${totals.ratio.toFixed(0)}%`}
              />
              <StatCard
                accentClass="text-[var(--route-local)]"
                detail={t("ledger.private_detail")}
                label={t("ledger.private_label")}
                value={t("ledger.megabytes", { value: stats.dataEgressProtectedMb.toFixed(1) })}
              />
              <StatCard
                accentClass="text-[var(--foreground)]"
                detail={t("ledger.savings_detail")}
                label={t("ledger.savings_label")}
                muted
                value={formatApproxCurrency(savings, language)}
              >
                <PricingBasis
                  isCustom={isCustomPricing}
                  onChange={updatePricing}
                  onReset={() =>
                    updatePricing({
                      inputPerMillion: DEFAULT_INPUT_USD_PER_MILLION,
                      outputPerMillion: DEFAULT_OUTPUT_USD_PER_MILLION,
                    })
                  }
                  pricing={pricing}
                  t={t}
                />
              </StatCard>
            </section>

            <section className="rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--background)] p-5">
              <div className="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
                <div>
                  <h2 className="text-sm font-semibold text-[var(--foreground)]">{t("ledger.mix_title")}</h2>
                  <p className="mt-1 text-xs font-medium text-[var(--foreground-muted)]">
                    {t("ledger.mix_summary", {
                      local: formatNumber(totals.local, language),
                      cloud: formatNumber(totals.cloud, language),
                    })}
                  </p>
                </div>
                <span className="rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--accent-background)] px-2.5 py-1 text-xs font-bold text-[var(--foreground-muted)]">
                  {t("ledger.mix_total", { count: formatNumber(totals.total, language) })}
                </span>
              </div>
              <div className="mt-5 h-3 overflow-hidden rounded-full bg-[var(--route-cloud-background)]">
                <div
                  className="h-full rounded-full bg-[var(--route-local)] transition-all duration-300"
                  style={{ width: `${totals.ratio}%` }}
                />
              </div>
              <div className="mt-3 grid grid-cols-2 gap-3 text-xs font-semibold text-[var(--foreground-muted)]">
                <span className="flex items-center gap-2">
                  <span aria-hidden="true" className="h-2 w-2 rounded-full bg-[var(--route-local)]" />
                  {t("ledger.mix_local")}
                </span>
                <span className="flex items-center justify-end gap-2">
                  {t("ledger.mix_cloud")}
                  <span aria-hidden="true" className="h-2 w-2 rounded-full bg-[var(--route-cloud)]" />
                </span>
              </div>
            </section>
          </>
        ) : null}
      </div>
    </main>
  );
}

function StatCard({
  accentClass,
  label,
  value,
  detail,
  muted = false,
  children,
}: {
  accentClass: string;
  label: string;
  value: string;
  detail: string;
  muted?: boolean;
  children?: React.ReactNode;
}) {
  return (
    <section className="flex min-h-36 flex-col justify-between rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-4">
      <div>
        <p className="text-[11px] font-bold uppercase tracking-wide text-[var(--foreground-muted)]">{label}</p>
        <p className={`mt-3 ${muted ? "text-2xl font-bold" : "text-3xl font-extrabold"} ${accentClass}`}>
          {value}
        </p>
      </div>
      <p className="mt-4 text-xs font-medium leading-5 text-[var(--foreground-muted)]">{detail}</p>
      {children}
    </section>
  );
}

function PricingBasis({
  pricing,
  isCustom,
  onChange,
  onReset,
  t,
}: {
  pricing: LedgerPricing;
  isCustom: boolean;
  onChange: (next: LedgerPricing) => void;
  onReset: () => void;
  t: (key: string, vars?: Record<string, string | number>) => string;
}) {
  const priceInputClass =
    "w-24 rounded-[var(--radius-xs)] border border-[var(--border-strong)] bg-[var(--background)] px-2 py-1 text-xs font-semibold text-[var(--foreground)] outline-none focus:border-[var(--accent)]";

  return (
    <details className="group mt-3 border-t border-[var(--border-soft)] pt-2 text-xs">
      <summary className="flex cursor-pointer select-none items-center justify-between gap-2 font-medium text-[var(--foreground-subtle)] outline-none transition-colors hover:text-[var(--foreground)]">
        <span>
          {isCustom
            ? t("ledger.pricing_basis_custom")
            : t("ledger.pricing_basis_default", {
                input: pricing.inputPerMillion.toString(),
                output: pricing.outputPerMillion.toString(),
              })}
        </span>
        <svg aria-hidden="true" className="h-3.5 w-3.5 shrink-0 transition-transform duration-150 group-open:rotate-90" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" viewBox="0 0 24 24">
          <path d="m9 6 6 6-6 6" />
        </svg>
      </summary>
      <div className="mt-2 flex flex-col gap-2">
        <label className="flex items-center justify-between gap-3">
          <span className="text-[var(--foreground-muted)]">{t("ledger.pricing_input")}</span>
          <input
            className={priceInputClass}
            inputMode="decimal"
            min={0}
            onChange={(event) =>
              onChange({ ...pricing, inputPerMillion: clampPrice(Number(event.target.value), 0) })
            }
            step="0.05"
            type="number"
            value={pricing.inputPerMillion}
          />
        </label>
        <label className="flex items-center justify-between gap-3">
          <span className="text-[var(--foreground-muted)]">{t("ledger.pricing_output")}</span>
          <input
            className={priceInputClass}
            inputMode="decimal"
            min={0}
            onChange={(event) =>
              onChange({ ...pricing, outputPerMillion: clampPrice(Number(event.target.value), 0) })
            }
            step="0.05"
            type="number"
            value={pricing.outputPerMillion}
          />
        </label>
        <p className="text-[11px] leading-4 text-[var(--foreground-subtle)]">{t("ledger.pricing_note")}</p>
        {isCustom && (
          <button
            className="self-start text-[11px] font-semibold text-[var(--accent)] transition-opacity hover:opacity-80"
            onClick={onReset}
            type="button"
          >
            {t("ledger.pricing_reset")}
          </button>
        )}
      </div>
    </details>
  );
}
