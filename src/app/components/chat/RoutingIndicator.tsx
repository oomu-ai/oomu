"use client";

import { useI18n } from "@/context/I18nContext";
import type { AutoRouteReadinessStatus } from "./autoRouteReadiness";

type RoutingIndicatorMode = "manual" | "history" | "auto";

export function modelIdentityIsOpaque(modelId: string | null | undefined) {
  const raw = modelId?.trim() ?? "";
  const normalized = raw.toLowerCase().replace(/[\s-]+/g, "_");
  return !raw
    || raw.startsWith("/")
    || raw.startsWith("~/")
    || raw.startsWith("file:")
    || normalized === "models"
    || normalized === "model"
    || normalized === "local"
    || normalized === "local_model"
    || normalized === "dynamic"
    || normalized === "unknown"
    || normalized === "selected_model";
}

export function compactExecutionModelLabel(
  modelId: string | null | undefined,
  unverifiedLabel = "",
) {
  const raw = modelId?.trim() ?? "";
  if (modelIdentityIsOpaque(raw)) return unverifiedLabel;

  const normalized = raw.toLowerCase();
  if (normalized.includes("gemma")) {
    const size = raw.match(/gemma[-_\s]*4[-_\s]*(e?\d+b)/i)?.[1];
    return size ? `Gemma 4 ${size.toUpperCase()}` : "Gemma 4";
  }
  if (normalized.includes("gemini")) {
    const match = raw.match(/gemini[-_\s]*([\d.]+)?[-_\s]*(flash|pro)?/i);
    const version = match?.[1] ? ` ${match[1]}` : "";
    const variant = match?.[2]
      ? ` ${match[2][0].toUpperCase()}${match[2].slice(1).toLowerCase()}`
      : "";
    return `Gemini${version}${variant}`;
  }
  if (normalized.includes("gpt")) {
    return raw
      .replace(/^openai[:/]/i, "")
      .replace(/\bgpt\b/i, "GPT")
      .replace(/-/g, " ");
  }
  return raw.length > 28 ? `${raw.slice(0, 25)}...` : raw;
}

function LocalRouteGlyph() {
  return (
    <svg aria-hidden="true" className="h-3 w-3 shrink-0 fill-current" viewBox="0 0 24 24">
      <path d="M7 7h10v10H7V7Zm2 2v6h6V9H9Z" />
      <path d="M9 2h2v3H9V2Zm4 0h2v3h-2V2ZM9 19h2v3H9v-3Zm4 0h2v3h-2v-3ZM2 9h3v2H2V9Zm0 4h3v2H2v-2Zm17-4h3v2h-3V9Zm0 4h3v2h-3v-2Z" />
    </svg>
  );
}

function CloudRouteGlyph() {
  return (
    <svg aria-hidden="true" className="h-3 w-3 shrink-0 fill-current" viewBox="0 0 24 24">
      <path d="M19.35 10.04A7.49 7.49 0 0 0 12 4a7.48 7.48 0 0 0-6.65 4.04A5.99 5.99 0 0 0 0 14c0 3.31 2.69 6 6 6h13c2.76 0 5-2.24 5-5 0-2.64-2.16-4.82-4.65-4.96Z" />
    </svg>
  );
}

export function AutoRouteGlyph() {
  return (
    <svg aria-hidden="true" className="h-3 w-3 shrink-0" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" viewBox="0 0 24 24">
      <path d="M4 7h11l-2.5-2.5M20 17H9l2.5 2.5" />
    </svg>
  );
}

function ActivityStatus({ value }: { value?: string | null }) {
  if (!value) return null;
  return (
    <span
      aria-atomic="true"
      aria-live="polite"
      className="mr-2 inline-flex max-w-[50%] items-center truncate rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--accent-background)] px-2 py-1 text-[11px] font-semibold text-[var(--foreground-muted)]"
      data-oomu-chat-status="true"
      id="oomu-chat-status"
      role="status"
      title={value}
    >
      {value}
    </span>
  );
}

export function RoutingIndicator({
  activityStatus,
  isLocal,
  modelId,
  mode,
  autoRouteStatus = "unknown",
  localModelId,
  cloudModelId,
  readinessGeneration,
}: {
  activityStatus?: string | null;
  isLocal: boolean;
  modelId: string;
  mode: RoutingIndicatorMode;
  autoRouteStatus?: AutoRouteReadinessStatus;
  localModelId?: string;
  cloudModelId?: string;
  classifierModelId?: string | null;
  readinessGeneration?: number;
}) {
  const { t } = useI18n();
  const unverifiedLocalLabel = t("sprint_301.route.on_device_model");
  const modelLabel = compactExecutionModelLabel(modelId, unverifiedLocalLabel);
  const chipClass =
    "inline-flex max-w-full select-none items-center gap-1.5 rounded-[var(--radius-sm)] border px-2 py-1 text-[11px] font-semibold";

  if (mode === "auto") {
    const localLabel = compactExecutionModelLabel(localModelId, unverifiedLocalLabel);
    const cloudLabel = cloudModelId?.trim()
      ? compactExecutionModelLabel(cloudModelId, t("chat.route.cloud_not_configured"))
      : t("chat.route.cloud_not_configured");
    const readinessKey = autoRouteStatus === "ready"
      ? "chat.route.ready"
      : autoRouteStatus === "loading" || autoRouteStatus === "recovering"
        ? "chat.route.preparing"
        : "chat.route.needs_attention";
    const readinessLabel = t(readinessKey);
    const classifierDetail = autoRouteStatus === "ready"
      ? t("sprint_301.route.details_ready")
      : autoRouteStatus === "loading" || autoRouteStatus === "recovering"
        ? t("sprint_301.route.details_preparing")
        : t("sprint_301.route.details_attention");
    return (
      <>
        <ActivityStatus value={activityStatus} />
        <div
          aria-description={classifierDetail}
          aria-label={t("chat.route.auto_pair_aria", {
            status: readinessLabel,
            local: localLabel,
            cloud: cloudLabel,
          })}
          className={`${chipClass} border-[var(--border-strong)] bg-[var(--background)] text-[var(--foreground-muted)]`}
          data-auto-route-status={autoRouteStatus}
          data-cloud-model-id={cloudModelId ?? ""}
          data-current-model-id={modelId}
          data-local-model-id={localModelId ?? ""}
          data-readiness-generation={readinessGeneration ?? 0}
          data-route-mode={mode}
          tabIndex={0}
          title={classifierDetail}
        >
          <AutoRouteGlyph />
          <span className="min-w-0 truncate">
            {readinessLabel} · {t("chat.route.local_pair", { model: localLabel })} · {t("chat.route.cloud_pair", { model: cloudLabel })}
          </span>
        </div>
      </>
    );
  }

  const routeWord = isLocal ? t("chat.route.sovereign") : t("chat.route.cloud");
  const ariaLabel = mode === "history"
    ? `${t("chat.route.last_reply")}: ${routeWord} · ${modelLabel}`
    : `${routeWord} · ${modelLabel}`;

  return (
    <>
      <ActivityStatus value={activityStatus} />
      <div
        aria-label={ariaLabel}
        className={`${chipClass} ${isLocal
          ? "border-[var(--route-local-border)] bg-[var(--route-local-background)] text-[var(--route-local)]"
          : "border-[var(--route-cloud-border)] bg-[var(--route-cloud-background)] text-[var(--route-cloud)] shadow-[var(--route-cloud-glow)]"}`}
        data-auto-route-status={autoRouteStatus}
        data-cloud-model-id={cloudModelId ?? ""}
        data-current-model-id={modelId}
        data-local-model-id={localModelId ?? ""}
        data-route-mode={mode}
      >
        {isLocal ? <LocalRouteGlyph /> : <CloudRouteGlyph />}
        <span className="min-w-0 truncate">
          {mode === "history" ? (
            <span className="font-medium opacity-80">{t("chat.route.last_reply")} · </span>
          ) : null}
          {routeWord} ({modelLabel})
        </span>
      </div>
    </>
  );
}
