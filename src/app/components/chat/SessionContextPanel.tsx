"use client";

import { invoke } from "@/lib/invoke";
import { useI18n } from "@/context/I18nContext";
import {
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";

const DEFAULT_AUTO_COMPACTION_PERCENT = 70;
const THRESHOLD_OPTIONS = [50, 60, 70, 80, 90] as const;

type ContextStatusWire = {
  estimatedTokensUsed?: number;
  estimated_tokens_used?: number;
  tokensTotal?: number;
  tokens_total?: number;
  workingBudgetTokens?: number;
  providerMaxTokens?: number;
  estimatedPercentageUsed?: number;
  estimated_percentage_used?: number;
  autoCompactionThresholdPercent?: number;
  auto_compaction_threshold_percent?: number;
  autoCompactionEnabled?: boolean;
  auto_compaction_enabled?: boolean;
  lastCompaction?: ContextCompactionResult | null;
};

export type ContextCompactionResult = {
  sessionId: string;
  beforeTokens: number;
  afterTokens: number;
  targetTokens: number;
  compactedMessageCount: number;
  preservedMessageCount: number;
  nextRequestTokens: number;
  thresholdPercent: number;
};

type ContextStatus = {
  usedTokens: number;
  workingBudgetTokens: number;
  providerMaxTokens: number;
  percentage: number;
};

export type SessionContextController = {
  status: ContextStatus | null;
  autoCompactionEnabled: boolean;
  thresholdPercent: number;
  lastCompaction: ContextCompactionResult | null;
  isCompacting: boolean;
  isSavingPolicy: boolean;
  errorKey: string | null;
  compactNow: () => Promise<void>;
  setAutoCompactionEnabled: (value: boolean) => Promise<void>;
  setThresholdPercent: (value: number) => Promise<void>;
};

function finiteNumber(value: unknown, fallback = 0) {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

function clampPercent(value: number) {
  return Math.min(100, Math.max(0, value));
}

function normalizeStatus(status: ContextStatusWire): ContextStatus {
  const usedTokens = Math.max(0, finiteNumber(
    status.estimatedTokensUsed ?? status.estimated_tokens_used,
  ));
  const workingBudgetTokens = Math.max(1, finiteNumber(
    status.workingBudgetTokens ?? status.tokensTotal ?? status.tokens_total,
    12_288,
  ));
  const providerMaxTokens = Math.max(
    workingBudgetTokens,
    finiteNumber(status.providerMaxTokens, workingBudgetTokens),
  );
  const suppliedPercentage = finiteNumber(
    status.estimatedPercentageUsed ?? status.estimated_percentage_used,
    usedTokens / workingBudgetTokens,
  );
  const percentage = suppliedPercentage > 1
    ? clampPercent(suppliedPercentage) / 100
    : clampPercent(suppliedPercentage * 100) / 100;
  return { usedTokens, workingBudgetTokens, providerMaxTokens, percentage };
}

function normalizedThreshold(value: unknown) {
  const numeric = finiteNumber(value, DEFAULT_AUTO_COMPACTION_PERCENT);
  return Math.min(90, Math.max(50, Math.round(numeric / 10) * 10));
}

function useContextPolicyActions({
  autoCompactionEnabled,
  isSavingPolicy,
  refresh,
  sessionId,
  setAutoCompactionEnabledState,
  setErrorKey,
  setIsSavingPolicy,
  setThreshold,
  thresholdPercent,
}: {
  autoCompactionEnabled: boolean;
  isSavingPolicy: boolean;
  refresh: () => Promise<void>;
  sessionId: string;
  setAutoCompactionEnabledState: (value: boolean) => void;
  setErrorKey: (value: string | null) => void;
  setIsSavingPolicy: (value: boolean) => void;
  setThreshold: (value: number) => void;
  thresholdPercent: number;
}) {
  const save = useCallback(async (
    nextEnabled: boolean,
    nextThreshold: number,
    rollback: () => void,
  ) => {
    const cleanSessionId = sessionId.trim();
    if (!cleanSessionId || isSavingPolicy) return;
    setIsSavingPolicy(true);
    setErrorKey(null);
    try {
      await invoke("save_session_context_policy", { request: {
        sessionId: cleanSessionId,
        autoCompactionEnabled: nextEnabled,
        autoCompactionThresholdPercent: nextThreshold,
      }});
      await refresh();
    } catch {
      rollback();
      setErrorKey("sprint_299.context.policy_failed");
    } finally {
      setIsSavingPolicy(false);
    }
  }, [isSavingPolicy, refresh, sessionId, setErrorKey, setIsSavingPolicy]);

  const setThresholdPercent = useCallback(async (value: number) => {
    const next = normalizedThreshold(value);
    const previous = thresholdPercent;
    setThreshold(next);
    await save(autoCompactionEnabled, next, () => setThreshold(previous));
  }, [autoCompactionEnabled, save, setThreshold, thresholdPercent]);

  const setAutoCompactionEnabled = useCallback(async (value: boolean) => {
    const previous = autoCompactionEnabled;
    setAutoCompactionEnabledState(value);
    await save(value, thresholdPercent, () => setAutoCompactionEnabledState(previous));
  }, [autoCompactionEnabled, save, setAutoCompactionEnabledState, thresholdPercent]);

  return { setAutoCompactionEnabled, setThresholdPercent };
}

export function useSessionContextController({
  onCompacted,
  refreshSignal,
  sessionId,
}: {
  onCompacted?: (result: ContextCompactionResult) => Promise<void> | void;
  refreshSignal: number;
  sessionId: string;
}): SessionContextController {
  const [status, setStatus] = useState<ContextStatus | null>(null);
  const [autoCompactionEnabled, setAutoCompactionEnabledState] = useState(true);
  const [thresholdPercent, setThreshold] = useState(DEFAULT_AUTO_COMPACTION_PERCENT);
  const [lastCompaction, setLastCompaction] = useState<ContextCompactionResult | null>(null);
  const [isCompacting, setIsCompacting] = useState(false);
  const [isSavingPolicy, setIsSavingPolicy] = useState(false);
  const [errorKey, setErrorKey] = useState<string | null>(null);
  const onCompactedRef = useRef(onCompacted);

  useEffect(() => {
    onCompactedRef.current = onCompacted;
  }, [onCompacted]);

  const refresh = useCallback(async () => {
    const cleanSessionId = sessionId.trim();
    if (!cleanSessionId) {
      setStatus(null);
      setErrorKey(null);
      return;
    }
    try {
      const response = await invoke<ContextStatusWire>("get_session_context_status", {
        sessionId: cleanSessionId,
        session_id: cleanSessionId,
      });
      setStatus(normalizeStatus(response));
      setAutoCompactionEnabledState(
        response.autoCompactionEnabled ?? response.auto_compaction_enabled ?? true,
      );
      setThreshold(normalizedThreshold(
        response.autoCompactionThresholdPercent
          ?? response.auto_compaction_threshold_percent,
      ));
      if ("lastCompaction" in response) {
        setLastCompaction(response.lastCompaction ?? null);
      }
      setErrorKey(null);
    } catch {
      setStatus(null);
      setErrorKey("sprint_299.context.unavailable");
    }
  }, [sessionId]);

  useEffect(() => {
    const initialRefresh = window.setTimeout(() => void refresh(), 0);
    const intervalId = window.setInterval(() => void refresh(), 15_000);
    const handleFocus = () => void refresh();
    window.addEventListener("focus", handleFocus);
    return () => {
      window.clearTimeout(initialRefresh);
      window.clearInterval(intervalId);
      window.removeEventListener("focus", handleFocus);
    };
  }, [refresh, refreshSignal, sessionId]);

  const runCompaction = useCallback(async () => {
    if (!sessionId.trim() || isCompacting) return;
    setIsCompacting(true);
    setErrorKey(null);
    try {
      const result = await invoke<ContextCompactionResult>("compact_chat_session", {
        request: { sessionId: sessionId.trim(), targetPercent: thresholdPercent },
      });
      setLastCompaction(result);
      await onCompactedRef.current?.(result);
      await refresh();
    } catch {
      setErrorKey("sprint_299.context.compaction_failed");
      throw new Error("context_compaction_failed");
    } finally {
      setIsCompacting(false);
    }
  }, [isCompacting, refresh, sessionId, thresholdPercent]);

  const { setAutoCompactionEnabled, setThresholdPercent } = useContextPolicyActions({
    autoCompactionEnabled, isSavingPolicy, refresh, sessionId,
    setAutoCompactionEnabledState, setErrorKey, setIsSavingPolicy,
    setThreshold, thresholdPercent,
  });

  return {
    status,
    autoCompactionEnabled,
    thresholdPercent,
    lastCompaction: lastCompaction?.sessionId === sessionId ? lastCompaction : null,
    isCompacting,
    isSavingPolicy,
    errorKey,
    compactNow: runCompaction,
    setAutoCompactionEnabled,
    setThresholdPercent,
  };
}

function formatTokens(value: number) {
  return Math.round(value).toLocaleString();
}

export function SessionContextPanel({
  controller,
  disabled = false,
}: {
  controller: SessionContextController;
  disabled?: boolean;
}) {
  const { t } = useI18n();
  const { status } = controller;
  const percent = Math.round((status?.percentage ?? 0) * 100);
  const noSession = !status && !controller.errorKey;

  return (
    <section className="rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--background)] p-4">
      <div className="flex items-center justify-between gap-3">
        <h2 className="text-xs font-semibold text-[var(--foreground)]">
          {t("sprint_299.context.title")}
        </h2>
        <span className="rounded-full bg-[var(--accent-background)] px-2 py-0.5 text-[11px] font-semibold text-[var(--foreground-muted)]">
          {t("sprint_299.context.percent_used", { percent })}
        </span>
      </div>
      <div className="mt-3 h-1.5 overflow-hidden rounded-full bg-[var(--border-soft)]">
        <div
          aria-hidden="true"
          className="h-full rounded-full bg-[var(--accent)] transition-[width] duration-300"
          style={{ width: `${Math.min(100, percent)}%` }}
        />
      </div>
      <p className="mt-3 text-xs leading-5 text-[var(--foreground-muted)]">
        {status
          ? t("sprint_299.context.usage", {
              used: formatTokens(status.usedTokens),
              budget: formatTokens(status.workingBudgetTokens),
            })
          : t("sprint_299.context.no_session")}
      </p>
      <p className="mt-2 text-[11px] leading-4 text-[var(--foreground-subtle)]">
        {t("sprint_299.context.keeps")}
      </p>
      {status && (
        <p className="mt-1 text-[11px] leading-4 text-[var(--foreground-subtle)]">
          {t("sprint_299.context.provider_max", {
            max: formatTokens(status.providerMaxTokens),
          })}
        </p>
      )}
      <div className="mt-4 flex items-center justify-between gap-3">
        <div>
          <p className="text-xs font-semibold text-[var(--foreground)]">
            {t("sprint_299.context.auto_toggle")}
          </p>
          <p className="mt-1 text-[11px] leading-4 text-[var(--foreground-subtle)]">
            {controller.autoCompactionEnabled
              ? t("sprint_299.context.auto_on")
              : t("sprint_299.context.auto_off")}
          </p>
        </div>
        <button
          aria-checked={controller.autoCompactionEnabled}
          aria-label={t("sprint_299.context.auto_toggle")}
          className={`relative h-7 w-12 shrink-0 rounded-full border transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${
            controller.autoCompactionEnabled
              ? "border-[var(--inverse-background)] bg-[var(--inverse-background)]"
              : "border-[var(--border-strong)] bg-[var(--background)]"
          }`}
          disabled={disabled || noSession || controller.isSavingPolicy}
          onClick={() => void controller.setAutoCompactionEnabled(!controller.autoCompactionEnabled)}
          role="switch"
          type="button"
        >
          <span
            className={`absolute left-0 top-1 h-5 w-5 rounded-full bg-[var(--background)] shadow-[0_1px_3px_rgba(15,23,42,0.24)] transition-transform ${
              controller.autoCompactionEnabled ? "translate-x-5" : "translate-x-1"
            }`}
          />
        </button>
      </div>
      <label className="mt-4 grid gap-1.5 text-xs font-medium text-[var(--foreground-muted)]">
        <span>{t("sprint_299.context.auto_label")}</span>
        <select
          aria-label={t("sprint_299.context.auto_label")}
          className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-sm text-[var(--foreground)]"
          disabled={disabled || noSession || controller.isSavingPolicy || !controller.autoCompactionEnabled}
          onChange={(event) => void controller.setThresholdPercent(Number(event.target.value))}
          value={controller.thresholdPercent}
        >
          {THRESHOLD_OPTIONS.map((value) => (
            <option key={value} value={value}>
              {t("sprint_299.context.threshold_option", { percent: value })}
            </option>
          ))}
        </select>
      </label>
      <button
        className="mt-3 flex h-9 w-full items-center justify-center rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-3 text-xs font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)] disabled:cursor-not-allowed disabled:opacity-50"
        disabled={disabled || noSession || controller.isCompacting}
        onClick={() => void controller.compactNow().catch(() => undefined)}
        type="button"
      >
        {controller.isCompacting
          ? t("sprint_299.context.compacting")
          : t("sprint_299.context.compact_now")}
      </button>
      {controller.lastCompaction && (
        <p className="mt-3 text-xs leading-5 text-[var(--foreground-muted)]" role="status">
          {t("sprint_299.context.measured_result", {
            before: formatTokens(controller.lastCompaction.beforeTokens),
            after: formatTokens(controller.lastCompaction.nextRequestTokens),
          })}
        </p>
      )}
      {controller.errorKey && (
        <p className="mt-3 text-xs leading-5 text-[var(--warning)]" role="alert">
          {t(controller.errorKey)}
        </p>
      )}
    </section>
  );
}
